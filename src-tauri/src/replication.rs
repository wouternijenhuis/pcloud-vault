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
    for entry in &objects {
        if !entry.is_folder && entry.name.ends_with(".obj") {
            if !names.insert(entry.name.clone()) {
                return Err("Duplicate encrypted object names found in pCloud. Sync paused to preserve your data.".into());
            }
            object_identity(&entry.name)?;
        }
    }
    Ok(objects)
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
            settings.diff_cursor,
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
    {
        let mut guard = locked(&s.vault)?;
        let vault = guard.as_mut().ok_or("Vault locked")?;
        vault.verify_header(&remote_header).map_err(|_| "The remote vault belongs to a different vault or its key envelope was altered. Sync paused without exchanging file data.".to_string())?;
        if !s.directory.join("pending-header.json").exists() {
            vault
                .adopt_header(&remote_header)
                .map_err(|e| e.to_string())?;
        }
    }
    if s.directory.join("pending-header.json").exists() {
        let pending: VaultHeader = serde_json::from_slice(
            &fs::read(s.directory.join("pending-header.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "Pending password update is invalid")?;
        locked(&s.vault)?
            .as_mut()
            .ok_or("Vault locked")?
            .adopt_header(&pending)
            .map_err(|e| e.to_string())?;
        publish_header(s)?;
    }
    let obsolete =
        last_sync.is_none_or(|last| now().saturating_sub(last) > 150 * 24 * 60 * 60 * 1000);
    let batch = client
        .diff(if obsolete { None } else { cursor }, false)
        .map_err(|e| e.to_string())?;
    // Reconcile the authoritative object listing after feed inspection. Names are opaque;
    // metadata authentication and causality are checked by the core before exposure.
    let remote = remote_objects(s, &client)?;
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
    let mut incoming = Vec::new();
    for entry in remote
        .iter()
        .filter(|m| !m.is_folder && m.name.ends_with(".obj") && !known.contains(&m.name))
    {
        if object_identity(&entry.name)?.1 == "chunk" {
            continue;
        }
        let object = decode(&entry.name, download(&client, entry.id, MAX_METADATA)?)?;
        // Keep metadata and causal history; chunks are fetched on demand. Avoid
        // retaining a whole account's file contents in the client's memory.
        if object.kind != "chunk" {
            incoming.push(object);
        }
        if incoming.len() >= 128 {
            locked(&s.vault)?
                .as_mut()
                .ok_or("Vault locked")?
                .ingest_objects(std::mem::take(&mut incoming))
                .map_err(|e| e.to_string())?;
        }
    }
    if !incoming.is_empty() {
        locked(&s.vault)?
            .as_mut()
            .ok_or("Vault locked")?
            .ingest_objects(incoming)
            .map_err(|e| e.to_string())?;
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
        settings.diff_cursor = Some(batch.cursor);
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
        vault.evict_cache().map_err(|e| e.to_string())?;
    }
    if !pending.is_empty() {
        s.log(
            "Encrypted changes uploaded and verified in pCloud.",
            "success",
        );
    }
    Ok(())
}

pub(crate) fn ensure_file(s: &Arc<Inner>, id: &str) -> AppResult<()> {
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
        return Ok(());
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
    Ok(())
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
