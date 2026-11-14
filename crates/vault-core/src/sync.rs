use crate::{
    crypto::valid_id,
    model::{Commit, Entry, EntryKind, LocalState, Operation, Resolved, ResolvedEntry},
    Error, Result,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type History = BTreeMap<String, BTreeSet<String>>;

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
            .retain(|(previous, _)| !history[id].contains(previous));
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
    let mut remaining = commits.clone();
    let mut history: History = BTreeMap::new();
    let mut records: BTreeMap<String, Raw> = BTreeMap::new();
    let mut heads = BTreeSet::new();
    loop {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|(_, commit)| {
                commit
                    .parents
                    .iter()
                    .all(|parent| history.contains_key(parent))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            break;
        }
        for id in ready {
            let commit = remaining.remove(&id).expect("ready commit exists");
            if commit.id != id
                || !valid_id(&id)
                || !valid_id(&commit.device_id)
                || commit.parents.len() > 4096
            {
                // Quarantined: it never enters history, so its descendants stay unresolved too.
                continue;
            }
            let mut ancestors = BTreeSet::new();
            for parent in &commit.parents {
                ancestors.insert(parent.clone());
                ancestors.extend(history[parent].iter().cloned());
            }
            history.insert(id.clone(), ancestors);
            // A semantically invalid commit (from a buggy or incompatible client) is skipped
            // deterministically instead of making the whole vault unreadable on every PC.
            let applied = apply_commit(&id, &commit, &history, &mut records);
            if applied.is_err() {
                history.remove(&id);
                continue;
            }
            for parent in &commit.parents {
                heads.remove(parent);
            }
            heads.insert(id.clone());
        }
    }
    resolve(commits, local, history, records, heads)
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
    commits: &BTreeMap<String, Commit>,
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
                .all(|(live_id, _)| history[trash_id].contains(live_id));
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
            sync_state: if commits.values().any(|commit| {
                commit.operation.affects_entry(id) && local.pending.contains(&commit.id)
            }) {
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
        accepted: history.into_keys().collect(),
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

fn derived_id(id: &str, domain: &str, revision: &str) -> String {
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
