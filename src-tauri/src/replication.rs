use crate::app::{atomic_write, locked, now, AppResult, Inner};
use pcloud_api::{Client, Metadata};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    sync::{atomic::Ordering, Arc},
};
use vault_core::{ReplicationObject, VaultHeader};

const STORE_NAME: &str = ".pcloud-vault";
const HEADER_NAME: &str = "vault.json";
const MAX_METADATA: usize = 64 * 1024 * 1024;

pub(crate) struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl BoundedWriter {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("Cloud object exceeds its permitted size"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn download(client: &Client, id: u64, max: usize) -> AppResult<Vec<u8>> {
    let mut writer = BoundedWriter::new(max);
    client
        .download_to(id, &mut writer)
        .map_err(|e| e.to_string())?;
    Ok(writer.into_inner())
}

pub(crate) fn discover(s: &Arc<Inner>) -> AppResult<()> {
    let client = s.client()?;
    let roots = client.list_folder(0).map_err(|e| e.to_string())?;
    let stores: Vec<_> = roots
        .into_iter()
        .filter(|m| m.is_folder && m.name.eq_ignore_ascii_case(STORE_NAME))
        .collect();
    if stores.len() > 1 {
        return Err(
            "Multiple vault storage folders found. Resolve duplicates in pCloud before connecting."
                .into(),
        );
    }
    let root = if let Some(root) = stores.into_iter().next() {
        root
    } else {
        client
            .create_folder(0, STORE_NAME)
            .map_err(|e| e.to_string())?
    };
    let files = client.list_folder(root.id).map_err(|e| e.to_string())?;
    let objects = if let Some(folder) = files.iter().find(|m| m.is_folder && m.name == "objects") {
        folder.clone()
    } else {
        client
            .create_folder(root.id, "objects")
            .map_err(|e| e.to_string())?
    };
    if let Some(header) = files.iter().find(|m| !m.is_folder && m.name == HEADER_NAME) {
        let bytes = download(&client, header.id, 64 * 1024)?;
        let remote: VaultHeader = serde_json::from_slice(&bytes).map_err(|_| {
            "Remote vault header is invalid. Its contents were not overwritten.".to_string()
        })?;
        let local_path = s.vault_path().join(HEADER_NAME);
        if local_path.exists() {
            let local =
                vault_core::Vault::read_header(&s.vault_path()).map_err(|e| e.to_string())?;
            if local.vault_id != remote.vault_id {
                return Err("This account already contains a different vault. The local and remote vaults were both preserved. Export the local vault before configuring this account on a clean installation.".into());
            }
        } else {
            atomic_write(&local_path, &bytes)?;
        }
    }
    {
        let mut settings = locked(&s.settings)?;
        if settings.remote_objects != Some(objects.id) {
            // A different objects folder needs a full listing, not the old feed position.
            settings.object_cursor = None;
        }
        settings.remote_folder = Some(root.id);
        settings.remote_objects = Some(objects.id);
    }
    s.save_settings()
}

fn object_name(id: &str, kind: &str) -> AppResult<String> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("Invalid encrypted object ID".into());
    }
    let prefix = match kind {
        "chunk" => "c",
        "manifest" => "m",
        "commit" => "h",
        _ => return Err("Invalid object kind".into()),
    };
    Ok(format!("{prefix}-{id}.obj"))
}
fn object_identity(name: &str) -> AppResult<(&str, &str)> {
    let stem = name.strip_suffix(".obj").ok_or("Invalid object filename")?;
    let (prefix, id) = stem.split_once('-').ok_or("Invalid object filename")?;
    let kind = match prefix {
        "c" => "chunk",
        "m" => "manifest",
        "h" => "commit",
        _ => return Err("Invalid object kind".into()),
    };
    object_name(id, kind)?;
    Ok((id, kind))
}
fn decode(name: &str, bytes: Vec<u8>) -> AppResult<ReplicationObject> {
    let (id, expected_kind) = object_identity(name)?;
    let kind = match bytes.first() {
        Some(1) => "chunk",
        Some(2) => "manifest",
        Some(3) => "commit",
        _ => return Err("Invalid encrypted object type".into()),
    };
    if kind != expected_kind {
        return Err("Object type does not match its encrypted envelope".into());
    }
    Ok(ReplicationObject {
        id: id.into(),
        kind: kind.into(),
        data: bytes[1..].to_vec(),
    })
}
fn encode(object: &ReplicationObject) -> AppResult<Vec<u8>> {
    let kind = match object.kind.as_str() {
        "chunk" => 1,
        "manifest" => 2,
        "commit" => 3,
        _ => return Err("Unknown encrypted object type".into()),
    };
    let mut bytes = Vec::with_capacity(object.data.len() + 1);
    bytes.push(kind);
    bytes.extend_from_slice(&object.data);
    Ok(bytes)
}

fn remote_objects(s: &Inner, client: &Client) -> AppResult<Vec<Metadata>> {
    let folder = locked(&s.settings)?
        .remote_objects
        .ok_or("No remote vault folder is configured")?;
    let objects = client.list_folder(folder).map_err(|e| e.to_string())?;
    let mut names = HashSet::new();
    let mut valid = Vec::with_capacity(objects.len());
    for entry in objects {
        if entry.is_folder || !entry.name.ends_with(".obj") {
            continue;
        }
        // Unexpected names (for example a leftover renamed duplicate) are ignored rather
        // than pausing sync for every PC; they are never treated as vault objects.
        if object_identity(&entry.name).is_err() {
            continue;
        }
        if !names.insert(entry.name.clone()) {
            return Err("Duplicate encrypted object names found in pCloud. Sync paused to preserve your data.".into());
        }
        valid.push(entry);
    }
    Ok(valid)
}

/// Ingests a batch; if it is rejected, retries each object alone so one invalid object
/// (from a buggy or incompatible client) cannot stop every later sync. Skipped objects are
/// retried on the next sync, when their dependencies may have arrived.
fn ingest_tolerantly(s: &Inner, objects: Vec<ReplicationObject>) -> AppResult<usize> {
    let mut guard = locked(&s.vault)?;
    let vault = guard.as_mut().ok_or("Vault locked")?;
    if vault.ingest_objects(objects.clone()).is_ok() {
        return Ok(0);
    }
    let skipped = objects
        .into_iter()
        .filter(|object| vault.ingest_objects(vec![object.clone()]).is_err())
        .count();
    if skipped > 0 {
        drop(guard);
        s.log(
            &format!("Skipped {skipped} encrypted object(s) that failed validation."),
            "info",
        );
    }
    Ok(skipped)
}

/// Lists every vault object. The feed position is taken first, so changes made during the
/// listing are replayed by the next incremental sync (objects are immutable, so replays are
/// harmless).
fn full_listing(s: &Inner, client: &Client) -> AppResult<(Vec<Metadata>, Option<u64>)> {
    let cursor = client.diff(None, false).ok().map(|batch| batch.cursor);
    Ok((remote_objects(s, client)?, cursor))
}

/// Folds change-feed events into the current set of candidate objects, keyed by name.
fn apply_feed_events(
    found: &mut std::collections::BTreeMap<String, Metadata>,
    events: Vec<pcloud_api::DiffEvent>,
    folder_id: u64,
) -> AppResult<()> {
    for event in events {
        match event.event.as_str() {
            // pCloud asks clients to rebuild their state; reconcile the full listing.
            "reset" => return Err("The change feed was reset.".into()),
            "createfile" | "modifyfile" | "deletefile" => {}
            _ => continue,
        }
        let Some(metadata) = event.metadata else {
            continue;
        };
        // Later events win: an object deleted or moved out of the folder after it was
        // created is no longer a candidate.
        found.remove(&metadata.name);
        if event.event != "deletefile"
            && !metadata.is_folder
            && metadata.parent_folder_id == folder_id
            && object_identity(&metadata.name).is_ok()
        {
            found.insert(metadata.name.clone(), metadata);
        }
    }
    Ok(())
}

/// Collects vault objects created since `cursor` from pCloud's change feed.
fn changed_objects(
    client: &Client,
    cursor: u64,
    folder_id: u64,
) -> AppResult<(Vec<Metadata>, Option<u64>)> {
    const MAX_PAGES: usize = 50;
    let mut position = cursor;
    let mut found = std::collections::BTreeMap::new();
    for _ in 0..MAX_PAGES {
        let batch = client
            .diff(Some(position), false)
            .map_err(|e| e.to_string())?;
        let complete = batch.events.len() < pcloud_api::DIFF_PAGE_SIZE;
        apply_feed_events(&mut found, batch.events, folder_id)?;
        position = batch.cursor;
        if complete {
            return Ok((found.into_values().collect(), Some(position)));
        }
    }
    Err("The change backlog is large; reconciling the full listing instead.".into())
}

pub(crate) struct SyncGuard<'a>(&'a Inner);
impl<'a> SyncGuard<'a> {
    pub fn acquire(inner: &'a Inner) -> AppResult<Self> {
        inner
            .syncing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Wait for the current sync to finish, then retry.".to_string())?;
        Ok(Self(inner))
    }
}
impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        self.0.syncing.store(false, Ordering::Release);
    }
}

pub(crate) fn sync(s: &Arc<Inner>) -> AppResult<()> {
    if s.syncing
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(());
    }
    let _guard = SyncGuard(s);
    if locked(&s.account)?.is_none() {
        return Ok(());
    }
    if locked(&s.vault)?.is_none() {
        return Ok(());
    }
    let client = s.client()?;
    if locked(&s.settings)?.remote_objects.is_none() {
        discover(s)?;
    }
    let (root_id, folder_id, cursor, last_sync) = {
        let settings = locked(&s.settings)?;
        (
            settings
                .remote_folder
                .ok_or("Missing remote vault folder")?,
            settings
                .remote_objects
                .ok_or("Missing remote objects folder")?,
            settings.object_cursor,
            settings.last_sync,
        )
    };
    let root_files = client.list_folder(root_id).map_err(|e| e.to_string())?;
    if !root_files
        .iter()
        .any(|m| m.name == HEADER_NAME && !m.is_folder)
    {
        let header = fs::read(s.vault_path().join(HEADER_NAME)).map_err(|e| e.to_string())?;
        client
            .upload_bytes_immutable(root_id, HEADER_NAME, header)
            .map_err(|e| e.to_string())?;
    }
    // Re-read the winning header even after creation: another PC may have
    // discovered the empty account concurrently with an independent vault.
    let published = client
        .list_folder(root_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| m.name == HEADER_NAME && !m.is_folder)
        .ok_or("Remote vault header was not published")?;
    let remote_header: VaultHeader =
        serde_json::from_slice(&download(&client, published.id, 64 * 1024)?)
            .map_err(|_| "Invalid remote vault header")?;
    remote_header.validate().map_err(|e| e.to_string())?;
    let pending_path = s.directory.join("pending-header.json");
    let mut republish = pending_path.exists();
    {
        let mut guard = locked(&s.vault)?;
        let vault = guard.as_mut().ok_or("Vault locked")?;
        vault.verify_header(&remote_header).map_err(|_| "The remote vault belongs to a different vault or its key envelope was altered. Sync paused without exchanging file data.".to_string())?;
        let current_salt = vault.header().salt.clone();
        if remote_header.salt != current_salt {
            if is_retired(s, &remote_header.salt)? {
                // Every wrapper this PC replaced stays retired, so an older (possibly
                // compromised) password cannot silently come back through the cloud.
                s.log(
                    "The cloud vault header was reverted to an older password. Restoring the current one.",
                    "info",
                );
                republish = true;
            } else {
                retire_header(s, &current_salt)?;
                vault
                    .adopt_header(&remote_header)
                    .map_err(|e| e.to_string())?;
                if pending_path.exists() {
                    fs::remove_file(&pending_path).map_err(|e| e.to_string())?;
                    republish = false;
                    s.log("The vault password was changed on another PC at the same time. That password now applies; the change made on this PC was not applied.", "info");
                }
            }
        }
    }
    if republish {
        publish_header(s)?;
    }
    let obsolete =
        last_sync.is_none_or(|last| now().saturating_sub(last) > 150 * 24 * 60 * 60 * 1000);
    // Incremental discovery replays pCloud's change feed from the saved cursor and only
    // considers objects created in the vault folder. First connection, an obsolete or
    // rejected cursor, or an oversized backlog fall back to a full listing.
    let (candidates, next_cursor) = match cursor.filter(|_| !obsolete) {
        Some(cursor) => match changed_objects(&client, cursor, folder_id) {
            Ok(found) => found,
            Err(_) => full_listing(s, &client)?,
        },
        None => full_listing(s, &client)?,
    };
    let known: HashSet<_> = {
        let vault = locked(&s.vault)?;
        vault
            .as_ref()
            .ok_or("Vault was locked during sync")?
            .object_ids()
            .into_iter()
            .map(|(id, kind)| object_name(&id, &kind))
            .collect::<AppResult<_>>()?
    };
    // Objects that fail validation keep the cursor in place, so they are retried next sync.
    let mut skipped = 0;
    let mut incoming = Vec::new();
    for entry in candidates.iter().filter(|m| !known.contains(&m.name)) {
        // Metadata and causal history only; chunks are fetched on demand so the client
        // never holds a whole account's file contents.
        if object_identity(&entry.name)?.1 == "chunk" {
            continue;
        }
        let Ok(object) =
            download(&client, entry.id, MAX_METADATA).and_then(|bytes| decode(&entry.name, bytes))
        else {
            s.log("Skipped an unreadable encrypted object in pCloud.", "info");
            skipped += 1;
            continue;
        };
        incoming.push(object);
        if incoming.len() >= 128 {
            skipped += ingest_tolerantly(s, std::mem::take(&mut incoming))?;
        }
    }
    if !incoming.is_empty() {
        skipped += ingest_tolerantly(s, incoming)?;
    }
    let pending = {
        let vault = locked(&s.vault)?;
        vault.as_ref().ok_or("Vault locked")?.pending_object_ids()
    };
    for id in &pending {
        let object = locked(&s.vault)?
            .as_ref()
            .ok_or("Vault locked")?
            .object(id)
            .map_err(|e| e.to_string())?;
        client
            .upload_bytes_immutable(folder_id, &object_name(id, &object.kind)?, encode(&object)?)
            .map_err(|e| e.to_string())?;
        locked(&s.vault)?
            .as_mut()
            .ok_or("Vault locked")?
            .acknowledge_objects(std::slice::from_ref(id))
            .map_err(|e| e.to_string())?;
    }
    {
        let mut settings = locked(&s.settings)?;
        // Skipped objects (invalid, or not downloadable right now) are retried by a full
        // listing on the next sync rather than replaying an ever-growing feed backlog.
        settings.object_cursor = if skipped == 0 { next_cursor } else { None };
        settings.last_sync = Some(now());
    }
    s.save_settings()?;
    let pinned: Vec<String> = {
        let vault = locked(&s.vault)?;
        let vault = vault.as_ref().ok_or("Vault locked")?;
        vault
            .all_entries()
            .into_iter()
            .filter(|e| {
                e.kind == vault_core::EntryKind::File && vault.is_pinned(&e.id) && !e.trashed
            })
            .map(|e| e.id)
            .collect()
    };
    for id in pinned {
        ensure_file(s, &id)?;
    }
    {
        let mut vault = locked(&s.vault)?;
        let vault = vault.as_mut().ok_or("Vault locked")?;
        vault
            .set_cache_limit_bytes(locked(&s.settings)?.cache_limit_gib * 1024 * 1024 * 1024)
            .map_err(|e| e.to_string())?;
        let in_use: std::collections::BTreeSet<String> =
            locked(&s.in_use)?.keys().cloned().collect();
        vault
            .evict_cache_protecting(&in_use)
            .map_err(|e| e.to_string())?;
    }
    if !pending.is_empty() {
        s.log(
            "Encrypted changes uploaded and verified in pCloud.",
            "success",
        );
    }
    Ok(())
}

/// Marks a vault entry as being read so cache eviction keeps its chunks. Hold it until the
/// read, preview or export that needed the hydrated data has finished.
pub(crate) struct InUse {
    inner: Arc<Inner>,
    id: String,
}
impl InUse {
    pub fn acquire(inner: &Arc<Inner>, id: &str) -> AppResult<Self> {
        *locked(&inner.in_use)?.entry(id.to_string()).or_insert(0) += 1;
        Ok(Self {
            inner: inner.clone(),
            id: id.to_string(),
        })
    }
}
impl Drop for InUse {
    fn drop(&mut self) {
        if let Ok(mut in_use) = self.inner.in_use.lock() {
            if let Some(count) = in_use.get_mut(&self.id) {
                *count -= 1;
                if *count == 0 {
                    in_use.remove(&self.id);
                }
            }
        }
    }
}

/// Downloads any missing encrypted objects for a file or folder. The returned guard keeps
/// them from being evicted while the caller reads them.
pub(crate) fn ensure_file(s: &Arc<Inner>, id: &str) -> AppResult<InUse> {
    // Register before checking what is missing: eviction takes the vault lock before
    // reading this registry, so it either sees this entry or runs before the check.
    let in_use = InUse::acquire(s, id)?;
    let missing = {
        let vault = locked(&s.vault)?;
        let vault = vault.as_ref().ok_or("Unlock your vault first")?;
        let entry = vault.entry(id).map_err(|e| e.to_string())?;
        let files = if entry.kind == vault_core::EntryKind::File {
            vec![id.to_string()]
        } else {
            let entries = vault.all_entries();
            entries
                .iter()
                .filter(|e| {
                    e.kind == vault_core::EntryKind::File && !e.trashed && {
                        let mut parent = e.parent_id.as_deref();
                        let mut inside = false;
                        while let Some(candidate) = parent {
                            if candidate == id {
                                inside = true;
                                break;
                            }
                            parent = entries
                                .iter()
                                .find(|e| e.id == candidate)
                                .and_then(|e| e.parent_id.as_deref());
                        }
                        inside
                    }
                })
                .map(|e| e.id.clone())
                .collect()
        };
        let mut missing = HashSet::new();
        for id in files {
            missing.extend(
                vault
                    .required_objects(&id)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .filter(|id| !vault.has_object(id)),
            );
        }
        missing.into_iter().collect::<Vec<_>>()
    };
    if missing.is_empty() {
        return Ok(in_use);
    }
    let client = s.client().map_err(|_| {
        "This file is not cached. Connect to pCloud to make it available.".to_string()
    })?;
    let remote = remote_objects(s, &client)?;
    for object_id in missing {
        let metadata = remote.iter().find(|m| !m.is_folder && object_identity(&m.name).is_ok_and(|(id, _)| id == object_id)).ok_or("Encrypted file data is unavailable in pCloud. Sync may still be running on another PC.")?;
        let object = decode(
            &metadata.name,
            download(&client, metadata.id, MAX_METADATA)?,
        )?;
        locked(&s.vault)?
            .as_mut()
            .ok_or("Vault locked")?
            .ingest_objects(vec![object])
            .map_err(|e| e.to_string())?;
    }
    Ok(in_use)
}

fn is_retired(s: &Inner, salt: &str) -> AppResult<bool> {
    Ok(locked(&s.settings)?
        .retired_header_salts
        .iter()
        .any(|retired| retired == salt))
}

/// Records a superseded password wrapper (identified by its unique random salt) so it is
/// never adopted again. Salts are public header fields, not secrets.
pub(crate) fn retire_header(s: &Inner, salt: &str) -> AppResult<()> {
    {
        let mut settings = locked(&s.settings)?;
        if settings
            .retired_header_salts
            .iter()
            .any(|retired| retired == salt)
        {
            return Ok(());
        }
        settings.retired_header_salts.push(salt.to_string());
    }
    s.save_settings()
}

pub(crate) fn publish_header(s: &Arc<Inner>) -> AppResult<()> {
    let client = s.client()?;
    let folder = locked(&s.settings)?
        .remote_folder
        .ok_or("Connect the remote vault before changing its password")?;
    let pending_path = s.directory.join("pending-header.json");
    let bytes = fs::read(if pending_path.exists() {
        pending_path.clone()
    } else {
        s.vault_path().join(HEADER_NAME)
    })
    .map_err(|e| e.to_string())?;
    let pending: VaultHeader =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid password wrapper")?;
    let current = client
        .list_folder(folder)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| m.name == HEADER_NAME && !m.is_folder)
        .ok_or("Remote vault header is unavailable")?;
    let remote: VaultHeader = serde_json::from_slice(&download(&client, current.id, 64 * 1024)?)
        .map_err(|_| "Invalid remote password wrapper")?;
    {
        let mut guard = locked(&s.vault)?;
        let vault = guard.as_mut().ok_or("Vault locked")?;
        vault
            .verify_header(&remote)
            .map_err(|_| "Remote vault identity changed; password update paused")?;
        // Overwrite only the wrapper this PC last adopted (or one it already retired);
        // anything else is a newer password from another PC that the next sync adopts.
        if remote.salt != vault.header().salt && !is_retired(s, &remote.salt)? {
            return Err(
                "The vault password was changed on another PC. Sync again to adopt it.".into(),
            );
        }
        vault.adopt_header(&pending).map_err(|e| e.to_string())?;
    }
    let length = bytes.len() as u64;
    let metadata = client
        .upload_reader(
            folder,
            HEADER_NAME,
            std::io::Cursor::new(bytes.clone()),
            length,
            false,
        )
        .map_err(|e| e.to_string())?;
    let check = download(&client, metadata.id, 64 * 1024)?;
    if bytes != check {
        return Err(
            "Remote password wrapper verification failed. The local vault is preserved.".into(),
        );
    }
    if pending_path.exists() {
        fs::remove_file(pending_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_limits_and_object_names_are_enforced() {
        let mut writer = BoundedWriter::new(4);
        writer.write_all(b"1234").unwrap();
        assert!(writer.write_all(b"5").is_err());
        assert_eq!(writer.into_inner(), b"1234");
        assert!(object_name("../../file", "chunk").is_err());
        assert!(object_name("A0000000000000000000000000000000", "chunk").is_err());
        assert!(object_name("a0000000000000000000000000000000", "chunk").is_ok());
    }
    #[test]
    fn feed_events_track_created_deleted_moved_and_reset_objects() {
        let name =
            |prefix: &str, digit: char| format!("{prefix}-{}.obj", digit.to_string().repeat(32));
        let event = |kind: &str, name: String, parent: u64| pcloud_api::DiffEvent {
            id: 1,
            event: kind.into(),
            time: 1,
            metadata: Some(Metadata {
                id: 9,
                name,
                is_folder: false,
                parent_folder_id: parent,
                size: 48,
                modified: 1,
            }),
        };
        let mut found = std::collections::BTreeMap::new();
        apply_feed_events(
            &mut found,
            vec![
                event("createfile", name("h", 'a'), 7),
                event("createfile", name("m", 'b'), 7),
                event("deletefile", name("m", 'b'), 7),
                event("createfile", name("c", 'c'), 7),
                event("modifyfile", name("c", 'c'), 8),
                event("createfile", name("h", 'd'), 8),
                event("createfile", "notes.txt".into(), 7),
            ],
            7,
        )
        .unwrap();
        assert_eq!(
            found.keys().cloned().collect::<Vec<_>>(),
            vec![name("h", 'a')]
        );
        let reset = pcloud_api::DiffEvent {
            id: 2,
            event: "reset".into(),
            time: 2,
            metadata: None,
        };
        assert!(apply_feed_events(&mut found, vec![reset], 7).is_err());
    }

    #[test]
    fn envelopes_roundtrip_without_exposing_names() {
        let object = ReplicationObject {
            id: "a0000000000000000000000000000000".into(),
            kind: "commit".into(),
            data: vec![5; 48],
        };
        let decoded = decode(
            &object_name(&object.id, &object.kind).unwrap(),
            encode(&object).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.id, object.id);
        assert_eq!(decoded.data, object.data);
        assert!(decode("bad.obj", vec![1]).is_err());
    }
}
