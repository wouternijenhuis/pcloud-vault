use crate::{
    crypto::valid_id,
    model::{Commit, Entry, EntryKind, LocalState, Operation, Resolved, ResolvedEntry},
    Error, Result,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Causal order of the accepted commits.
///
/// Commits are grouped into linear segments: every commit after the first one of a segment
/// has exactly one parent, the commit before it. The ancestors of a commit are therefore the
/// earlier commits of its segment plus the first commit's parents and their ancestors, so an
/// ancestry check walks segments instead of storing every commit's full ancestor set.
/// Generations (one more than the highest parent) prune walks that cannot reach the target.
#[derive(Default)]
struct History {
    positions: BTreeMap<String, Position>,
    segments: Vec<Segment>,
}

#[derive(Clone, Copy)]
struct Position {
    segment: usize,
    index: usize,
    generation: u64,
}

struct Segment {
    parents: Vec<String>,
    first_generation: u64,
    len: usize,
}

impl History {
    /// Adds a commit whose parents are all accepted already.
    fn insert(&mut self, id: &str, parents: &[String]) {
        let parents: BTreeSet<&String> = parents.iter().collect();
        let generation = 1 + parents
            .iter()
            .map(|parent| self.positions[parent.as_str()].generation)
            .max()
            .unwrap_or(0);
        let extends = match parents.iter().next() {
            Some(parent) if parents.len() == 1 => {
                let position = self.positions[parent.as_str()];
                (position.index + 1 == self.segments[position.segment].len)
                    .then_some(position.segment)
            }
            _ => None,
        };
        let position = match extends {
            Some(segment) => {
                self.segments[segment].len += 1;
                Position {
                    segment,
                    index: self.segments[segment].len - 1,
                    generation,
                }
            }
            None => {
                self.segments.push(Segment {
                    parents: parents.into_iter().cloned().collect(),
                    first_generation: generation,
                    len: 1,
                });
                Position {
                    segment: self.segments.len() - 1,
                    index: 0,
                    generation,
                }
            }
        };
        self.positions.insert(id.to_string(), position);
    }

    /// Takes back the most recent [`History::insert`], for a commit that could not be applied.
    fn remove_last(&mut self, id: &str) {
        let position = self
            .positions
            .remove(id)
            .expect("the removed commit was inserted last");
        if position.index == 0 {
            self.segments.pop();
        } else {
            self.segments[position.segment].len -= 1;
        }
    }

    /// Whether `ancestor` is a strict ancestor of `descendant`.
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let (Some(target), Some(start)) =
            (self.positions.get(ancestor), self.positions.get(descendant))
        else {
            return false;
        };
        // Each item stands for a segment's commits before `limit` and all of their ancestors.
        let mut pending = vec![(start.segment, start.index)];
        let mut explored: BTreeMap<usize, usize> = BTreeMap::new();
        let mut parents_queued: BTreeSet<usize> = BTreeSet::new();
        while let Some((segment, limit)) = pending.pop() {
            if explored
                .get(&segment)
                .is_some_and(|&previous| previous >= limit)
            {
                continue;
            }
            explored.insert(segment, limit);
            if target.segment == segment && target.index < limit {
                return true;
            }
            // Everything before a segment has a lower generation than its first commit.
            let data = &self.segments[segment];
            if target.generation < data.first_generation && parents_queued.insert(segment) {
                for parent in &data.parents {
                    let position = self.positions[parent.as_str()];
                    pending.push((position.segment, position.index + 1));
                }
            }
        }
        false
    }

    fn into_accepted(self) -> BTreeSet<String> {
        self.positions.into_keys().collect()
    }
}

#[derive(Debug, Clone)]
struct Register<T> {
    values: Vec<(String, T)>,
}
impl<T: Clone> Register<T> {
    fn new(id: &str, value: T) -> Self {
        Self {
            values: vec![(id.into(), value)],
        }
    }
    fn set(&mut self, id: &str, value: T, history: &History) {
        self.values
            .retain(|(previous, _)| !history.is_ancestor(previous, id));
        self.values.push((id.into(), value));
        self.values.sort_by(|a, b| a.0.cmp(&b.0));
    }
    fn winner(&self) -> &(String, T) {
        self.values.last().expect("a register always has a value")
    }
}

struct Raw {
    kind: EntryKind,
    names: Register<String>,
    parents: Register<Option<String>>,
    contents: Register<(Option<String>, u64)>,
    trash: Register<bool>,
    live: Register<()>,
    modified: u64,
}

pub(crate) struct Reduction {
    pub entries: Resolved,
    pub heads: Vec<String>,
    pub accepted: BTreeSet<String>,
}

pub(crate) fn reduce(commits: &BTreeMap<String, Commit>, local: &LocalState) -> Result<Reduction> {
    let mut history = History::default();
    let mut records: BTreeMap<String, Raw> = BTreeMap::new();
    let mut heads = BTreeSet::new();
    // Commits are applied in rounds: each round takes, in id order, every commit whose
    // parents were all accepted in earlier rounds.
    let mut waiting: BTreeMap<&str, usize> = BTreeMap::new();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut ready: BTreeSet<&str> = BTreeSet::new();
    for (id, commit) in commits {
        let parents: BTreeSet<&str> = commit.parents.iter().map(String::as_str).collect();
        if parents.is_empty() {
            ready.insert(id);
            continue;
        }
        waiting.insert(id, parents.len());
        for parent in parents {
            children.entry(parent).or_default().push(id);
        }
    }
    while !ready.is_empty() {
        let mut next = BTreeSet::new();
        for id in ready {
            let commit = &commits[id];
            if commit.id != id
                || !valid_id(id)
                || !valid_id(&commit.device_id)
                || commit.parents.len() > 4096
            {
                // Quarantined: it never enters history, so its descendants stay unresolved too.
                continue;
            }
            history.insert(id, &commit.parents);
            // A semantically invalid commit (from a buggy or incompatible client) is skipped
            // deterministically instead of making the whole vault unreadable on every PC.
            if apply_commit(id, commit, &history, &mut records).is_err() {
                history.remove_last(id);
                continue;
            }
            for parent in &commit.parents {
                heads.remove(parent);
            }
            heads.insert(id.to_string());
            for child in children.get(id).into_iter().flatten() {
                let missing = waiting.get_mut(child).expect("child waits for parents");
                *missing -= 1;
                if *missing == 0 {
                    next.insert(*child);
                }
            }
        }
        ready = next;
    }
    let pending_entries: BTreeSet<&str> = commits
        .values()
        .filter(|commit| local.pending.contains(&commit.id))
        .flat_map(|commit| commit.operation.affected_entries())
        .collect();
    resolve(&pending_entries, local, history, records, heads)
}

fn apply_commit(
    id: &str,
    commit: &Commit,
    history: &History,
    records: &mut BTreeMap<String, Raw>,
) -> Result<()> {
    {
        {
            let id = id.to_string();
            let entry_id = commit.operation.entry_id().to_string();
            if !valid_id(&entry_id) {
                return Err(Error::InvalidFormat("invalid entry id".into()));
            }
            if let Operation::Create {
                parent,
                name,
                kind,
                manifest,
                size,
                ..
            } = &commit.operation
            {
                crate::validate_name(name)?;
                if let Some(existing) = records.get_mut(&entry_id) {
                    if existing.kind != *kind {
                        return Err(Error::InvalidFormat("inconsistent file identity".into()));
                    }
                    existing.names.set(&id, name.clone(), history);
                    existing.parents.set(&id, parent.clone(), history);
                    existing
                        .contents
                        .set(&id, (manifest.clone(), *size), history);
                    existing.live.set(&id, (), history);
                    existing.modified = existing.modified.max(commit.modified);
                    return Ok(());
                }
                records.insert(
                    entry_id,
                    Raw {
                        kind: *kind,
                        names: Register::new(&id, name.clone()),
                        parents: Register::new(&id, parent.clone()),
                        contents: Register::new(&id, (manifest.clone(), *size)),
                        trash: Register::new(&id, false),
                        live: Register::new(&id, ()),
                        modified: commit.modified,
                    },
                );
                return Ok(());
            }
            if let Operation::Relocate {
                parent,
                name,
                displaced,
                ..
            } = &commit.operation
            {
                crate::validate_name(name)?;
                if parent.as_deref().is_some_and(|parent| !valid_id(parent)) {
                    return Err(Error::InvalidFormat("invalid relocation parent".into()));
                }
                let source = records.get(&entry_id).ok_or_else(|| {
                    Error::InvalidFormat("relocation source does not exist".into())
                })?;
                if let Some(target_id) = displaced {
                    if target_id == &entry_id {
                        return Err(Error::InvalidFormat(
                            "relocation cannot replace itself".into(),
                        ));
                    }
                    let target = records.get(target_id).ok_or_else(|| {
                        Error::InvalidFormat("relocation target does not exist".into())
                    })?;
                    if source.kind != EntryKind::File || target.kind != EntryKind::File {
                        return Err(Error::InvalidFormat(
                            "folder replacement is unsupported".into(),
                        ));
                    }
                }
                let source = records.get_mut(&entry_id).expect("source was validated");
                source.names.set(&id, name.clone(), history);
                source.parents.set(&id, parent.clone(), history);
                source.live.set(&id, (), history);
                source.modified = source.modified.max(commit.modified);
                if let Some(target_id) = displaced {
                    let target = records.get_mut(target_id).expect("target was validated");
                    target.trash.set(&id, true, history);
                    target.modified = target.modified.max(commit.modified);
                }
                return Ok(());
            }
            let Some(record) = records.get_mut(&entry_id) else {
                return Err(Error::InvalidFormat(
                    "operation references an unknown entry".into(),
                ));
            };
            match &commit.operation {
                Operation::Write { manifest, size, .. } => {
                    if record.kind != EntryKind::File {
                        return Err(Error::InvalidFormat("folder content write".into()));
                    }
                    record
                        .contents
                        .set(&id, (Some(manifest.clone()), *size), history);
                    record.live.set(&id, (), history);
                }
                Operation::Rename { name, .. } => {
                    crate::validate_name(name)?;
                    record.names.set(&id, name.clone(), history);
                    record.live.set(&id, (), history);
                }
                Operation::Move { parent, .. } => {
                    record.parents.set(&id, parent.clone(), history);
                    record.live.set(&id, (), history);
                }
                Operation::Trash { trashed, .. } => {
                    record.trash.set(&id, *trashed, history);
                    if !trashed {
                        record.live.set(&id, (), history);
                    }
                }
                Operation::Create { .. } | Operation::Relocate { .. } => unreachable!(),
            }
            record.modified = record.modified.max(commit.modified);
        }
    }
    Ok(())
}

fn resolve(
    pending_entries: &BTreeSet<&str>,
    local: &LocalState,
    history: History,
    records: BTreeMap<String, Raw>,
    heads: BTreeSet<String>,
) -> Result<Reduction> {
    let mut entries = Resolved::new();
    for (id, record) in &records {
        let (trash_id, deleted) = record.trash.winner();
        let trashed = *deleted
            && record
                .live
                .values
                .iter()
                .all(|(live_id, _)| history.is_ancestor(live_id, trash_id));
        let (content_commit, (manifest, size)) = record.contents.winner();
        let base = Entry {
            id: id.clone(),
            name: record.names.winner().1.clone(),
            parent_id: record.parents.winner().1.clone(),
            kind: record.kind,
            size: *size,
            modified: record.modified,
            starred: local.favorites.contains(id),
            pinned: local.pinned.contains(id),
            trashed,
            sync_state: if pending_entries.contains(id.as_str()) {
                "local".into()
            } else {
                "synced".into()
            },
        };
        entries.insert(
            id.clone(),
            ResolvedEntry {
                entry: base.clone(),
                manifest: manifest.clone(),
                original: true,
            },
        );
        // Every concurrent content value remains visible; derived IDs make replicas converge.
        for (commit_id, (other_manifest, other_size)) in &record.contents.values {
            if commit_id == content_commit || other_manifest == manifest {
                continue;
            }
            let conflict_id = derived_id(id, "content", commit_id);
            if records.contains_key(&conflict_id) {
                continue;
            }
            let mut conflict = base.clone();
            conflict.id = conflict_id.clone();
            conflict.name = conflict_name(&base.name, &format!("conflict {}", &commit_id[..8]));
            conflict.size = *other_size;
            conflict.trashed = false;
            entries.insert(
                conflict_id,
                ResolvedEntry {
                    entry: conflict,
                    manifest: other_manifest.clone(),
                    original: false,
                },
            );
        }
        // Concurrent location/name proposals are recoverable aliases, rather than silent loss.
        for (commit_id, name) in &record.names.values {
            if name == &base.name {
                continue;
            }
            add_location_conflict(
                &mut entries,
                &records,
                id,
                commit_id,
                name,
                base.parent_id.clone(),
                &base,
                manifest,
            );
        }
        for (commit_id, parent) in &record.parents.values {
            if parent == &base.parent_id {
                continue;
            }
            add_location_conflict(
                &mut entries,
                &records,
                id,
                commit_id,
                &base.name,
                parent.clone(),
                &base,
                manifest,
            );
        }
    }
    // Concurrent moves can produce cycles even when each originating move was valid.
    // Deterministically recover the lexicographically smallest member at the root.
    loop {
        let mut recovered = false;
        for id in entries.keys().cloned().collect::<Vec<_>>() {
            let mut chain: Vec<String> = Vec::new();
            let mut current = Some(id.clone());
            while let Some(node) = current {
                if let Some(index) = chain.iter().position(|seen| seen == &node) {
                    let root = chain[index..].iter().min().expect("cycle nonempty").clone();
                    entries
                        .get_mut(&root)
                        .expect("cycle member exists")
                        .entry
                        .parent_id = None;
                    recovered = true;
                    break;
                }
                chain.push(node.clone());
                current = entries
                    .get(&node)
                    .and_then(|entry| entry.entry.parent_id.clone());
            }
            if recovered {
                break;
            }
        }
        if !recovered {
            break;
        }
    }
    // Missing, deleted or non-folder parents recover their live children at the root.
    let invalid_parents: Vec<String> = entries
        .iter()
        .filter(|(_, entry)| {
            !entry.entry.trashed
                && entry.entry.parent_id.as_ref().is_some_and(|parent| {
                    entries
                        .get(parent)
                        .is_none_or(|p| p.entry.kind != EntryKind::Folder || p.entry.trashed)
                })
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in invalid_parents {
        entries.get_mut(&id).expect("entry exists").entry.parent_id = None;
    }
    // Case-insensitive names cannot collide on the mounted Windows drive.
    let mut used: BTreeSet<(Option<String>, String)> = BTreeSet::new();
    for (id, resolved) in &mut entries {
        if resolved.entry.trashed {
            continue;
        }
        let mut key = (
            resolved.entry.parent_id.clone(),
            resolved.entry.name.to_lowercase(),
        );
        if used.contains(&key) {
            let original_name = resolved.entry.name.clone();
            // Among N used names at most N candidates can be occupied. The ordinal
            // makes this bounded even for deliberately chosen conflict-like names.
            for ordinal in 0..=used.len() {
                let suffix = if ordinal == 0 {
                    format!("conflict {id}")
                } else {
                    format!("conflict {id}-{ordinal}")
                };
                resolved.entry.name = conflict_name(&original_name, &suffix);
                key.1 = resolved.entry.name.to_lowercase();
                if !used.contains(&key) {
                    break;
                }
            }
        }
        used.insert(key);
    }
    Ok(Reduction {
        entries,
        heads: heads.into_iter().collect(),
        accepted: history.into_accepted(),
    })
}

#[allow(clippy::too_many_arguments)]
fn add_location_conflict(
    entries: &mut Resolved,
    records: &BTreeMap<String, Raw>,
    id: &str,
    commit_id: &str,
    name: &str,
    parent: Option<String>,
    base: &Entry,
    manifest: &Option<String>,
) {
    let conflict_id = derived_id(id, "location", commit_id);
    if records.contains_key(&conflict_id) {
        return;
    }
    let mut alias = base.clone();
    alias.id = conflict_id.clone();
    alias.name = conflict_name(name, &format!("location conflict {}", &commit_id[..8]));
    alias.parent_id = parent;
    alias.trashed = false;
    entries.insert(
        conflict_id,
        ResolvedEntry {
            entry: alias,
            manifest: manifest.clone(),
            original: false,
        },
    );
}

pub(crate) fn derived_id(id: &str, domain: &str, revision: &str) -> String {
    hex::encode(&Sha256::digest(format!("pcloud-vault/{domain}/{id}/{revision}").as_bytes())[..16])
}

fn conflict_name(name: &str, suffix: &str) -> String {
    let allowance = 240_usize.saturating_sub(suffix.encode_utf16().count() + 3);
    let mut remaining = allowance;
    let prefix: String = name
        .chars()
        .take_while(|character| {
            let units = character.len_utf16();
            if units > remaining {
                false
            } else {
                remaining -= units;
                true
            }
        })
        .collect();
    format!("{prefix} ({suffix})")
}

#[cfg(test)]
pub(crate) fn conflict_name_for_reference(name: &str, suffix: &str) -> String {
    conflict_name(name, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntryKind;

    /// Small deterministic generator, so a failing history can be replayed from its seed.
    struct Random(u64);
    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }
    }

    fn hex_id(random: &mut Random) -> String {
        format!("{:016x}{:016x}", random.next(), random.next())
    }

    fn random_history(seed: u64, size: usize) -> (BTreeMap<String, Commit>, LocalState) {
        let mut random = Random(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let devices: Vec<String> = (0..3).map(|_| hex_id(&mut random)).collect();
        let entries: Vec<String> = (0..6).map(|_| hex_id(&mut random)).collect();
        let names = ["a.txt", "A.TXT", "b", "Folder", "notes.md", "x"];
        let mut ids: Vec<String> = Vec::new();
        let mut commits = BTreeMap::new();
        for _ in 0..size {
            let id = hex_id(&mut random);
            let mut parents = Vec::new();
            if !ids.is_empty() && random.below(10) != 0 {
                for _ in 0..1 + random.below(3) {
                    // Mostly recent commits, so histories form long chains with some forks.
                    let back = if random.below(4) == 0 {
                        random.below(ids.len())
                    } else {
                        random.below(ids.len().min(3))
                    };
                    parents.push(ids[ids.len() - 1 - back].clone());
                }
            }
            if random.below(40) == 0 {
                parents.push(hex_id(&mut random)); // a parent that never arrives
            }
            let entry = entries[random.below(entries.len())].clone();
            let other = entries[random.below(entries.len())].clone();
            let name = names[random.below(names.len())].to_string();
            let parent = (random.below(3) != 0).then(|| other.clone());
            let manifest = format!("{:032x}", random.below(4));
            let size = random.below(3) as u64;
            let operation = match random.below(7) {
                0 | 1 => Operation::Create {
                    id: entry,
                    parent,
                    name,
                    kind: if random.below(3) == 0 {
                        EntryKind::Folder
                    } else {
                        EntryKind::File
                    },
                    manifest: Some(manifest),
                    size,
                },
                2 => Operation::Write {
                    id: entry,
                    manifest,
                    size,
                },
                3 => Operation::Rename { id: entry, name },
                4 => Operation::Move { id: entry, parent },
                5 => Operation::Relocate {
                    id: entry,
                    parent,
                    name,
                    displaced: (random.below(2) == 0).then_some(other),
                },
                _ => Operation::Trash {
                    id: entry,
                    trashed: random.below(3) != 0,
                },
            };
            let commit = Commit {
                id: if random.below(60) == 0 {
                    "not-a-valid-id".into()
                } else {
                    id.clone()
                },
                parents,
                device_id: devices[random.below(devices.len())].clone(),
                modified: random.below(1000) as u64,
                operation,
            };
            ids.push(id.clone());
            commits.insert(id, commit);
        }
        let local = LocalState {
            device_id: devices[0].clone(),
            pending: ids
                .iter()
                .filter(|_| random.below(5) == 0)
                .cloned()
                .collect(),
            favorites: entries.iter().take(1).cloned().collect(),
            pinned: entries.iter().skip(1).take(1).cloned().collect(),
            cache_limit_bytes: 0,
            known_commits: BTreeSet::new(),
        };
        (commits, local)
    }

    #[test]
    fn reducer_matches_the_previous_implementation_on_random_histories() {
        for seed in 1..=3000_u64 {
            let size = 1 + (seed as usize % 60);
            let (commits, local) = random_history(seed, size);
            let current = reduce(&commits, &local).unwrap();
            let reference = crate::sync_reference::reduce(&commits, &local).unwrap();
            assert_eq!(
                current.heads, reference.heads,
                "heads differ for seed {seed}"
            );
            assert_eq!(
                current.accepted, reference.accepted,
                "accepted commits differ for seed {seed}"
            );
            assert_eq!(
                format!("{:?}", current.entries),
                format!("{:?}", reference.entries),
                "entries differ for seed {seed}"
            );
        }
    }

    #[test]
    fn ancestry_follows_forks_and_merges() {
        let mut history = History::default();
        let ids: Vec<String> = (0..7).map(|n| format!("{n:032x}")).collect();
        let parents = |list: &[usize]| list.iter().map(|&n| ids[n].clone()).collect::<Vec<_>>();
        // 0 - 1 - 2 ----- 5 - 6
        //      \- 3 - 4 -/
        history.insert(&ids[0], &[]);
        history.insert(&ids[1], &parents(&[0]));
        history.insert(&ids[2], &parents(&[1]));
        history.insert(&ids[3], &parents(&[1]));
        history.insert(&ids[4], &parents(&[3]));
        history.insert(&ids[5], &parents(&[2, 4]));
        history.insert(&ids[6], &parents(&[5]));
        let is = |a: usize, b: usize| history.is_ancestor(&ids[a], &ids[b]);
        assert!(is(0, 6) && is(3, 6) && is(4, 5) && is(1, 4) && is(2, 5));
        assert!(!is(2, 4) && !is(4, 2) && !is(3, 2) && !is(6, 0) && !is(5, 5));
        history.remove_last(&ids[6]);
        assert!(!history.is_ancestor(&ids[0], &ids[6]));
    }

    #[test]
    fn ancestry_matches_full_ancestor_sets_on_random_dags() {
        for seed in 1..=400_u64 {
            let mut random = Random(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1);
            let mut history = History::default();
            let mut closure: Vec<(String, BTreeSet<String>)> = Vec::new();
            for n in 0..1 + (seed as usize % 80) {
                let id = format!("{n:032x}");
                let mut parents = Vec::new();
                if !closure.is_empty() {
                    for _ in 0..random.below(4) {
                        parents.push(closure[random.below(closure.len())].0.clone());
                    }
                }
                let mut ancestors = BTreeSet::new();
                for parent in &parents {
                    ancestors.insert(parent.clone());
                    let (_, above) = closure.iter().find(|(id, _)| id == parent).unwrap();
                    ancestors.extend(above.iter().cloned());
                }
                history.insert(&id, &parents);
                if random.below(5) == 0 {
                    history.remove_last(&id);
                    continue;
                }
                closure.push((id, ancestors));
            }
            for (descendant, ancestors) in &closure {
                for (candidate, _) in &closure {
                    assert_eq!(
                        history.is_ancestor(candidate, descendant),
                        ancestors.contains(candidate),
                        "seed {seed}: is {candidate} an ancestor of {descendant}?"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod measurements {
    use super::*;
    use crate::model::EntryKind;

    /// `cargo test --release -p vault-core reduce_scaling -- --ignored --nocapture`
    #[test]
    #[ignore = "timing measurement"]
    fn reduce_scaling_measurement() {
        for count in [10_000_usize, 50_000] {
            let mut commits = BTreeMap::new();
            let mut previous: Option<String> = None;
            for n in 0..count {
                let id = format!("{:032x}", n as u128 * 0x9e37_79b9_7f4a_7c15 + 1);
                let entry = format!("{:032x}", n % 2000 + 1);
                let operation = if n < 2000 {
                    Operation::Create {
                        id: entry,
                        parent: None,
                        name: format!("file {n}"),
                        kind: EntryKind::File,
                        manifest: None,
                        size: 0,
                    }
                } else {
                    Operation::Rename {
                        id: entry,
                        name: format!("renamed {n}"),
                    }
                };
                commits.insert(
                    id.clone(),
                    Commit {
                        id: id.clone(),
                        parents: previous.iter().cloned().collect(),
                        device_id: format!("{:032x}", 7),
                        modified: n as u64,
                        operation,
                    },
                );
                previous = Some(id);
            }
            let local = LocalState {
                device_id: format!("{:032x}", 7),
                pending: BTreeSet::new(),
                favorites: BTreeSet::new(),
                pinned: BTreeSet::new(),
                cache_limit_bytes: 0,
                known_commits: BTreeSet::new(),
            };
            let started = std::time::Instant::now();
            let reduction = reduce(&commits, &local).unwrap();
            println!(
                "{count} commits (2000 files, then renames): reduce {:?}, {} accepted",
                started.elapsed(),
                reduction.accepted.len()
            );
        }
    }
}
