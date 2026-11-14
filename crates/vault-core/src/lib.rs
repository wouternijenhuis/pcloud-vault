mod crypto;
mod model;
mod store;
mod sync;

use crypto::{decode_fixed, derive_key, new_id, open, random, seal, valid_id};
pub use crypto::{KdfParams, VaultHeader, CHUNK_SIZE, FORMAT_VERSION};
use fs2::FileExt;
use model::{ChunkRef, Commit, LocalState, Manifest, Operation, Resolved};
pub use model::{Entry, EntryKind, ReplicationObject};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use store::{read_limited, recover_checkpoint, write_atomic, write_new, ObjectStore};
use zeroize::Zeroizing;

pub type Result<T> = std::result::Result<T, Error>;
pub const HEADER_FILENAME: &str = "vault.json";
const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("authentication failed: wrong password, key, or damaged encrypted data")]
    Authentication,
    #[error("invalid vault format: {0}")]
    InvalidFormat(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("file or folder does not exist")]
    NotFound,
    #[error("a file or folder with that name already exists")]
    AlreadyExists,
    #[error("vault is already open in another process")]
    InUse,
    #[error("encrypted object {0} is not cached; reconnect to download it")]
    OfflineUnavailable(String),
    #[error("previously observed history is missing; refusing a possible rollback")]
    RollbackDetected,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

pub struct Vault {
    root: PathBuf,
    header: VaultHeader,
    key: Zeroizing<[u8; 32]>,
    local: LocalState,
    store: ObjectStore,
    entries: Resolved,
    commits: BTreeMap<String, Commit>,
    heads: Vec<String>,
    _process_lock: File,
}

impl Vault {
    pub fn exists(root: &Path) -> bool {
        root.join(HEADER_FILENAME).exists() || root.join("vault.previous").exists()
    }

    pub fn create(root: &Path, password: &str) -> Result<Self> {
        Self::create_with_kdf(root, password, KdfParams::default())
    }

    fn create_with_kdf(root: &Path, password: &str, kdf: KdfParams) -> Result<Self> {
        if Self::exists(root) {
            return Err(Error::AlreadyExists);
        }
        let key = Zeroizing::new(random::<32>());
        let header = VaultHeader::new(password, &key, kdf)?;
        Self::attach_with_key(root, &header, *key)
    }

    pub fn read_header(root: &Path) -> Result<VaultHeader> {
        recover_checkpoint(&root.join(HEADER_FILENAME))?;
        let header: VaultHeader =
            serde_json::from_slice(&read_limited(&root.join(HEADER_FILENAME), 16 * 1024)?)?;
        header.validate()?;
        Ok(header)
    }

    pub fn open(root: &Path, password: &str) -> Result<Self> {
        let header = Self::read_header(root)?;
        let key = header.unwrap(password)?;
        Self::open_inner(root, header, *key)
    }

    pub fn from_key(root: &Path, key: [u8; 32]) -> Result<Self> {
        Self::open_inner(root, Self::read_header(root)?, key)
    }

    pub fn attach(root: &Path, header: &VaultHeader, password: &str) -> Result<Self> {
        Self::attach_with_key(root, header, *header.unwrap(password)?)
    }

    pub fn attach_with_key(root: &Path, header: &VaultHeader, key: [u8; 32]) -> Result<Self> {
        header.verify_key(&key)?;
        fs::create_dir_all(root)?;
        if Self::exists(root) {
            return Err(Error::AlreadyExists);
        }
        write_new(
            &root.join(HEADER_FILENAME),
            &serde_json::to_vec_pretty(header)?,
        )?;
        Self::open_inner(root, header.clone(), key)
    }

    fn open_inner(root: &Path, header: VaultHeader, key: [u8; 32]) -> Result<Self> {
        let key = Zeroizing::new(key);
        header.verify_key(&key)?;
        let process_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("vault.lock"))?;
        process_lock
            .try_lock_exclusive()
            .map_err(|_| Error::InUse)?;
        let store = ObjectStore::open(root, &header.vault_id, &key)?;
        let local_path = root.join("local.bin");
        recover_checkpoint(&local_path)?;
        let local = if local_path.exists() {
            let bytes = read_limited(&local_path, 64 * 1024 * 1024)?;
            let checkpoint_key = Zeroizing::new(derive_key(&key, b"local-checkpoint"));
            let plaintext =
                Zeroizing::new(open(&checkpoint_key, &bytes, header.vault_id.as_bytes())?);
            serde_json::from_slice(&plaintext)?
        } else {
            LocalState::default()
        };
        let mut vault = Self {
            root: root.to_path_buf(),
            header,
            key,
            local,
            store,
            entries: Resolved::new(),
            commits: BTreeMap::new(),
            heads: Vec::new(),
            _process_lock: process_lock,
        };
        vault.rebuild()?;
        // Immutable commit files are the durable journal. Recover locally published commits
        // if a crash occurred before the encrypted checkpoint recorded their pending IDs.
        for commit in vault.commits.values() {
            if commit.device_id == vault.local.device_id
                && !vault.local.known_commits.contains(&commit.id)
            {
                vault.local.pending.insert(commit.id.clone());
                if let Some(manifest_id) = commit.operation.manifest_id() {
                    vault.local.pending.insert(manifest_id.into());
                    let manifest: Manifest = vault.store.metadata("manifest", manifest_id)?;
                    vault
                        .local
                        .pending
                        .extend(manifest.chunks.iter().map(|chunk| chunk.id.clone()));
                }
            }
        }
        vault.save_local()?;
        vault.rebuild()?;
        Ok(vault)
    }

    pub fn vault_id(&self) -> &str {
        &self.header.vault_id
    }
    pub fn header(&self) -> &VaultHeader {
        &self.header
    }
    pub fn key_bytes(&self) -> [u8; 32] {
        *self.key
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn device_id(&self) -> &str {
        &self.local.device_id
    }

    pub fn list(&self, parent_id: Option<&str>, include_trashed: bool) -> Vec<Entry> {
        self.entries
            .values()
            .filter(|resolved| {
                resolved.entry.parent_id.as_deref() == parent_id
                    && (include_trashed || !resolved.entry.trashed)
            })
            .map(|resolved| resolved.entry.clone())
            .collect()
    }
    pub fn all_entries(&self) -> Vec<Entry> {
        self.entries
            .values()
            .map(|resolved| resolved.entry.clone())
            .collect()
    }
    pub fn entry(&self, id: &str) -> Result<Entry> {
        self.entries
            .get(id)
            .map(|resolved| resolved.entry.clone())
            .ok_or(Error::NotFound)
    }

    pub fn create_folder(&mut self, parent_id: Option<&str>, name: &str) -> Result<Entry> {
        self.validate_destination(parent_id, name, None)?;
        let id = new_id();
        self.commit(Operation::Create {
            id: id.clone(),
            parent: parent_id.map(str::to_string),
            name: name.into(),
            kind: EntryKind::Folder,
            manifest: None,
            size: 0,
        })?;
        self.entry(&id)
    }

    pub fn import_file<R: Read>(
        &mut self,
        parent_id: Option<&str>,
        name: &str,
        mut reader: R,
    ) -> Result<Entry> {
        self.validate_destination(parent_id, name, None)?;
        let id = new_id();
        let manifest_id = self.store_reader(&id, &mut reader)?;
        let manifest: Manifest = self.store.metadata("manifest", &manifest_id)?;
        self.commit(Operation::Create {
            id: id.clone(),
            parent: parent_id.map(str::to_string),
            name: name.into(),
            kind: EntryKind::File,
            manifest: Some(manifest_id),
            size: manifest.size,
        })?;
        self.entry(&id)
    }

    pub fn read(&self, id: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let manifest = self.manifest(id)?;
        self.read_manifest(&manifest, offset, buffer)
    }

    fn read_manifest(&self, manifest: &Manifest, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if offset >= manifest.size || buffer.is_empty() {
            return Ok(0);
        }
        let amount = (manifest.size - offset).min(buffer.len() as u64) as usize;
        let mut copied = 0;
        while copied < amount {
            let position = offset + copied as u64;
            let index = (position / CHUNK_SIZE as u64) as usize;
            let within = (position % CHUNK_SIZE as u64) as usize;
            let chunk = self.decrypt_chunk(manifest, index)?;
            let take = (amount - copied).min(chunk.len() - within);
            buffer[copied..copied + take].copy_from_slice(&chunk[within..within + take]);
            copied += take;
        }
        Ok(copied)
    }

    pub fn write(&mut self, id: &str, offset: u64, data: &[u8]) -> Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        self.materialize(id)?;
        let old = self.manifest(id)?;
        let size = old.size.max(
            offset
                .checked_add(data.len() as u64)
                .ok_or_else(|| Error::InvalidInput("file offset overflow".into()))?,
        );
        self.rewrite(id, &old, size, Some((offset, data)))?;
        Ok(data.len())
    }

    pub fn truncate(&mut self, id: &str, size: u64) -> Result<()> {
        self.materialize(id)?;
        let old = self.manifest(id)?;
        self.rewrite(id, &old, size, None)
    }

    fn rewrite(
        &mut self,
        id: &str,
        old: &Manifest,
        size: u64,
        overlay: Option<(u64, &[u8])>,
    ) -> Result<()> {
        if size > MAX_FILE_SIZE {
            return Err(Error::InvalidInput(
                "maximum supported file size is 1 TiB".into(),
            ));
        }
        let mut manifest = Manifest {
            file_id: id.into(),
            version_id: new_id(),
            size,
            key: random::<32>(),
            chunks: Vec::new(),
        };
        let count = size.div_ceil(CHUNK_SIZE as u64);
        for index in 0..count {
            let start = index * CHUNK_SIZE as u64;
            let length = (size - start).min(CHUNK_SIZE as u64) as usize;
            let mut plaintext = Zeroizing::new(vec![0_u8; length]);
            if start < old.size {
                self.read_manifest(old, start, &mut plaintext)?;
            }
            if let Some((offset, data)) = overlay {
                let overlap_start = start.max(offset);
                let overlap_end = (start + length as u64).min(offset + data.len() as u64);
                if overlap_start < overlap_end {
                    plaintext[(overlap_start - start) as usize..(overlap_end - start) as usize]
                        .copy_from_slice(
                            &data[(overlap_start - offset) as usize
                                ..(overlap_end - offset) as usize],
                        );
                }
            }
            self.append_chunk(&mut manifest, &plaintext)?;
        }
        let manifest_id = self.finish_manifest(&manifest)?;
        self.commit(Operation::Write {
            id: id.into(),
            manifest: manifest_id,
            size,
        })
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<()> {
        let entry = self.entry(id)?;
        self.validate_destination(entry.parent_id.as_deref(), name, Some(id))?;
        self.materialize(id)?;
        self.commit(Operation::Rename {
            id: id.into(),
            name: name.into(),
        })
    }
    pub fn move_entry(&mut self, id: &str, parent_id: Option<&str>) -> Result<()> {
        let entry = self.entry(id)?;
        self.validate_destination(parent_id, &entry.name, Some(id))?;
        let mut parent = parent_id.map(str::to_string);
        while let Some(ancestor) = parent {
            if ancestor == id {
                return Err(Error::InvalidInput(
                    "a folder cannot be moved inside itself".into(),
                ));
            }
            parent = self.entry(&ancestor)?.parent_id;
        }
        self.materialize(id)?;
        self.commit(Operation::Move {
            id: id.into(),
            parent: parent_id.map(str::to_string),
        })
    }
    pub fn relocate(
        &mut self,
        id: &str,
        parent_id: Option<&str>,
        name: &str,
        replace: bool,
    ) -> Result<()> {
        let source = self.entry(id)?;
        if source.trashed {
            return Err(Error::NotFound);
        }
        validate_name(name)?;
        if let Some(parent) = parent_id {
            let destination = self.entry(parent)?;
            if destination.kind != EntryKind::Folder || destination.trashed {
                return Err(Error::InvalidInput(
                    "destination is not a live folder".into(),
                ));
            }
        }
        let mut parent = parent_id.map(str::to_string);
        while let Some(ancestor) = parent {
            if ancestor == id {
                return Err(Error::InvalidInput(
                    "a folder cannot be moved inside itself".into(),
                ));
            }
            parent = self.entry(&ancestor)?.parent_id;
        }
        let displaced = self
            .entries
            .values()
            .find(|resolved| {
                !resolved.entry.trashed
                    && resolved.entry.id != id
                    && resolved.entry.parent_id.as_deref() == parent_id
                    && resolved.entry.name.to_lowercase() == name.to_lowercase()
            })
            .map(|resolved| resolved.entry.clone());
        if let Some(target) = &displaced {
            if !replace {
                return Err(Error::AlreadyExists);
            }
            if source.kind != EntryKind::File || target.kind != EntryKind::File {
                return Err(Error::InvalidInput(
                    "replacing folders is unsupported".into(),
                ));
            }
        }
        // All destination checks precede materialization and journal publication.
        // Alias materialization preserves data but never renames/trashes either side.
        self.preserve_conflicts()?;
        self.materialize(id)?;
        if let Some(target) = &displaced {
            self.materialize(&target.id)?;
        }
        self.commit_raw(Operation::Relocate {
            id: id.into(),
            parent: parent_id.map(str::to_string),
            name: name.into(),
            displaced: displaced.map(|target| target.id),
        })
    }
    pub fn set_trashed(&mut self, id: &str, trashed: bool) -> Result<()> {
        let entry = self.entry(id)?;
        let mut descendants: Vec<(usize, String)> = Vec::new();
        if entry.kind == EntryKind::Folder {
            for candidate in self.entries.values() {
                let mut parent = candidate.entry.parent_id.clone();
                let mut depth = 1;
                while let Some(ancestor) = parent {
                    if ancestor == id {
                        descendants.push((depth, candidate.entry.id.clone()));
                        break;
                    }
                    parent = self
                        .entries
                        .get(&ancestor)
                        .and_then(|resolved| resolved.entry.parent_id.clone());
                    depth += 1;
                }
            }
        }
        // Trash leaves first, restore parents first, preserving the subtree in history.
        descendants.sort();
        if !trashed {
            self.materialize(id)?;
            self.commit(Operation::Trash {
                id: id.into(),
                trashed,
            })?;
        } else {
            descendants.reverse();
        }
        for (_, child_id) in descendants {
            self.materialize(&child_id)?;
            self.commit(Operation::Trash {
                id: child_id,
                trashed,
            })?;
        }
        if trashed {
            self.materialize(id)?;
            self.commit(Operation::Trash {
                id: id.into(),
                trashed,
            })?;
        }
        Ok(())
    }
    pub fn set_favorite(&mut self, id: &str, value: bool) -> Result<()> {
        self.entry(id)?;
        if value {
            self.local.favorites.insert(id.into());
        } else {
            self.local.favorites.remove(id);
        }
        self.save_local()?;
        self.rebuild()
    }
    pub fn set_pinned(&mut self, id: &str, value: bool) -> Result<()> {
        self.entry(id)?;
        if value {
            self.local.pinned.insert(id.into());
        } else {
            self.local.pinned.remove(id);
        }
        self.save_local()?;
        self.rebuild()
    }

    pub fn export_file(&self, id: &str, destination: &Path) -> Result<()> {
        let manifest = self.manifest(id)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let result = (|| {
            for index in 0..manifest.chunks.len() {
                output.write_all(&self.decrypt_chunk(&manifest, index)?)?;
            }
            output.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            drop(output);
            let _ = fs::remove_file(destination);
        }
        result
    }

    pub fn change_password(&mut self, old_password: &str, new_password: &str) -> Result<()> {
        let header = self.prepare_password_change(old_password, new_password)?;
        self.adopt_header(&header)
    }

    pub fn prepare_password_change(
        &self,
        old_password: &str,
        new_password: &str,
    ) -> Result<VaultHeader> {
        let old_key = self.header.unwrap(old_password)?;
        if old_key.as_ref() != self.key.as_ref() {
            return Err(Error::Authentication);
        }
        let mut header = self.header.clone();
        header.rewrap(new_password, &self.key)?;
        Ok(header)
    }

    pub fn verify_header(&self, header: &VaultHeader) -> Result<()> {
        if header.vault_id != self.vault_id() {
            return Err(Error::Authentication);
        }
        header.verify_key(&self.key)
    }

    pub fn adopt_header(&mut self, header: &VaultHeader) -> Result<()> {
        self.verify_header(header)?;
        write_atomic(
            &self.root.join(HEADER_FILENAME),
            &serde_json::to_vec_pretty(header)?,
        )?;
        self.header = header.clone();
        Ok(())
    }
    pub fn recovery_key(&self) -> String {
        format!("PV1-{}-{}", self.vault_id(), hex::encode(self.key.as_ref()))
    }
    pub fn open_with_recovery(root: &Path, recovery: &str) -> Result<Self> {
        let header = Self::read_header(root)?;
        let mut parts = recovery.trim().split('-');
        if parts.next() != Some("PV1") || parts.next() != Some(header.vault_id.as_str()) {
            return Err(Error::Authentication);
        }
        let key = decode_fixed::<32>(parts.next().ok_or(Error::Authentication)?)?;
        if parts.next().is_some() {
            return Err(Error::Authentication);
        }
        Self::from_key(root, key)
    }
    pub fn export_recovery(&self, destination: &Path) -> Result<()> {
        write_new(destination, self.recovery_key().as_bytes())
    }

    pub fn object_ids(&self) -> Vec<(String, String)> {
        self.store
            .kinds
            .iter()
            .map(|(id, kind)| (id.clone(), kind.clone()))
            .collect()
    }
    pub fn object(&self, id: &str) -> Result<ReplicationObject> {
        self.store.object(id)
    }
    pub fn has_object(&self, id: &str) -> bool {
        self.store.kinds.contains_key(id)
    }
    pub fn pending_object_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .local
            .pending
            .iter()
            .filter(|id| self.store.kinds.contains_key(*id))
            .cloned()
            .collect();
        ids.sort_by_key(|id| {
            (
                match self.store.kinds[id].as_str() {
                    "chunk" => 0,
                    "manifest" => 1,
                    _ => 2,
                },
                id.clone(),
            )
        });
        // Causal parent commits must precede their descendants.
        let mut output = Vec::new();
        let mut pending_commits: BTreeSet<String> = ids
            .iter()
            .filter(|id| self.store.kinds[*id] == "commit")
            .cloned()
            .collect();
        output.extend(
            ids.into_iter()
                .filter(|id| self.store.kinds[id] != "commit"),
        );
        while !pending_commits.is_empty() {
            let ready: Vec<String> = pending_commits
                .iter()
                .filter(|id| {
                    self.commits.get(*id).is_none_or(|commit| {
                        commit
                            .parents
                            .iter()
                            .all(|parent| !pending_commits.contains(parent))
                    })
                })
                .cloned()
                .collect();
            if ready.is_empty() {
                break;
            }
            for id in ready {
                pending_commits.remove(&id);
                output.push(id);
            }
        }
        output
    }
    pub fn export_objects(&self) -> Result<Vec<ReplicationObject>> {
        self.object_ids()
            .iter()
            .map(|(id, _)| self.object(id))
            .collect()
    }
    pub fn pending_objects(&self) -> Result<Vec<ReplicationObject>> {
        self.pending_object_ids()
            .iter()
            .map(|id| self.object(id))
            .collect()
    }
    pub fn acknowledge_objects(&mut self, ids: &[String]) -> Result<()> {
        for id in ids {
            self.local.pending.remove(id);
            if self.commits.contains_key(id) {
                self.local.known_commits.insert(id.clone());
            }
        }
        self.save_local()?;
        self.rebuild()
    }
    pub fn ingest_objects(&mut self, objects: Vec<ReplicationObject>) -> Result<()> {
        // An incoming causal successor may replace a currently visible register value.
        // Persist its exposed conflict copies before integrating that successor.
        self.preserve_conflicts()?;
        for object in &objects {
            self.store.validate_metadata(object)?;
            self.store.insert(object)?;
        }
        self.rebuild()?;
        self.local
            .known_commits
            .extend(self.commits.keys().cloned());
        self.save_local()
    }
    pub fn required_objects(&self, id: &str) -> Result<Vec<String>> {
        let manifest_id = self
            .entries
            .get(id)
            .ok_or(Error::NotFound)?
            .manifest
            .as_ref()
            .ok_or_else(|| Error::InvalidInput("entry is a folder".into()))?;
        let manifest = self.manifest(id)?;
        let mut objects = vec![manifest_id.clone()];
        objects.extend(manifest.chunks.iter().map(|chunk| chunk.id.clone()));
        Ok(objects)
    }
    pub fn missing_objects(&self) -> Vec<String> {
        let mut missing = BTreeSet::new();
        for commit in self.commits.values() {
            if let Some(id) = commit.operation.manifest_id() {
                if !self.has_object(id) {
                    missing.insert(id.into());
                } else if let Ok(manifest) = self.store.metadata::<Manifest>("manifest", id) {
                    missing.extend(
                        manifest
                            .chunks
                            .iter()
                            .filter(|chunk| !self.has_object(&chunk.id))
                            .map(|chunk| chunk.id.clone()),
                    );
                }
            }
        }
        missing.into_iter().collect()
    }
    pub fn cache_bytes(&self) -> Result<u64> {
        let mut bytes = 0_u64;
        for (id, _) in self.object_ids() {
            bytes = bytes.saturating_add(
                fs::metadata(self.root.join("objects").join(format!("{id}.obj")))?.len(),
            );
        }
        Ok(bytes)
    }
    pub fn cache_limit_bytes(&self) -> u64 {
        self.local.cache_limit_bytes
    }
    pub fn set_cache_limit_bytes(&mut self, value: u64) -> Result<()> {
        if value < CHUNK_SIZE as u64 {
            return Err(Error::InvalidInput(
                "cache limit must hold at least one chunk".into(),
            ));
        }
        self.local.cache_limit_bytes = value;
        self.save_local()
    }
    pub fn evict_cache(&mut self) -> Result<u64> {
        let mut protected = self.local.pending.clone();
        for id in &self.local.pending {
            let manifest_id = match self.store.kinds.get(id).map(String::as_str) {
                Some("manifest") => Some(id.as_str()),
                Some("commit") => self
                    .commits
                    .get(id)
                    .and_then(|commit| commit.operation.manifest_id()),
                _ => None,
            };
            if let Some(manifest_id) = manifest_id {
                let manifest: Manifest = self.store.metadata("manifest", manifest_id)?;
                protected.extend(manifest.chunks.iter().map(|chunk| chunk.id.clone()));
            }
        }
        for resolved in self
            .entries
            .values()
            .filter(|resolved| self.is_pinned(&resolved.entry.id))
        {
            if let Some(id) = &resolved.manifest {
                let manifest: Manifest = self.store.metadata("manifest", id)?;
                protected.extend(manifest.chunks.iter().map(|chunk| chunk.id.clone()));
            }
        }
        let mut bytes = self.cache_bytes()?;
        let before = bytes;
        for (id, kind) in self.object_ids() {
            if bytes <= self.local.cache_limit_bytes {
                break;
            }
            if kind != "chunk" || protected.contains(&id) {
                continue;
            }
            let path = self.root.join("objects").join(format!("{id}.obj"));
            let size = fs::metadata(&path)?.len();
            fs::remove_file(path)?;
            self.store.kinds.remove(&id);
            bytes = bytes.saturating_sub(size);
        }
        Ok(before - bytes)
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        let mut current = Some(id.to_string());
        while let Some(id) = current {
            if self.local.pinned.contains(&id) {
                return true;
            }
            current = self
                .entries
                .get(&id)
                .and_then(|resolved| resolved.entry.parent_id.clone());
        }
        false
    }

    fn store_reader<R: Read>(&mut self, file_id: &str, reader: &mut R) -> Result<String> {
        let mut manifest = Manifest {
            file_id: file_id.into(),
            version_id: new_id(),
            size: 0,
            key: random::<32>(),
            chunks: Vec::new(),
        };
        loop {
            let mut buffer = Zeroizing::new(vec![0_u8; CHUNK_SIZE]);
            let mut length = 0;
            while length < CHUNK_SIZE {
                match reader.read(&mut buffer[length..]) {
                    Ok(0) => break,
                    Ok(count) => length += count,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            if length == 0 {
                break;
            }
            manifest.size = manifest
                .size
                .checked_add(length as u64)
                .ok_or_else(|| Error::InvalidInput("file too large".into()))?;
            if manifest.size > MAX_FILE_SIZE {
                return Err(Error::InvalidInput(
                    "maximum supported file size is 1 TiB".into(),
                ));
            }
            self.append_chunk(&mut manifest, &buffer[..length])?;
        }
        self.finish_manifest(&manifest)
    }
    fn append_chunk(&mut self, manifest: &mut Manifest, plaintext: &[u8]) -> Result<()> {
        let id = new_id();
        let index = manifest.chunks.len();
        let aad = chunk_aad(
            self.vault_id(),
            &manifest.file_id,
            &manifest.version_id,
            &id,
            index,
            plaintext.len(),
        )?;
        self.store.insert(&ReplicationObject {
            id: id.clone(),
            kind: "chunk".into(),
            data: seal(&manifest.key, plaintext, &aad)?,
        })?;
        self.local.pending.insert(id.clone());
        manifest.chunks.push(ChunkRef {
            id,
            length: plaintext.len() as u32,
        });
        Ok(())
    }
    fn finish_manifest(&mut self, manifest: &Manifest) -> Result<String> {
        let id = new_id();
        self.store.put_metadata("manifest", &id, manifest)?;
        self.local.pending.insert(id.clone());
        Ok(id)
    }
    fn manifest(&self, id: &str) -> Result<Manifest> {
        let resolved = self.entries.get(id).ok_or(Error::NotFound)?;
        let manifest_id = resolved
            .manifest
            .as_ref()
            .ok_or_else(|| Error::InvalidInput("entry is a folder".into()))?;
        let manifest: Manifest = self.store.metadata("manifest", manifest_id)?;
        self.validate_manifest(&manifest)?;
        if manifest.size != resolved.entry.size {
            return Err(Error::Authentication);
        }
        Ok(manifest)
    }
    fn validate_manifest(&self, manifest: &Manifest) -> Result<()> {
        if !valid_id(&manifest.file_id)
            || !valid_id(&manifest.version_id)
            || manifest.size > MAX_FILE_SIZE
            || manifest.chunks.len() as u64 != manifest.size.div_ceil(CHUNK_SIZE as u64)
        {
            return Err(Error::InvalidFormat("invalid file manifest".into()));
        }
        let mut unique = BTreeSet::new();
        for (index, chunk) in manifest.chunks.iter().enumerate() {
            let expected =
                (manifest.size - index as u64 * CHUNK_SIZE as u64).min(CHUNK_SIZE as u64) as u32;
            if !valid_id(&chunk.id) || chunk.length != expected || !unique.insert(&chunk.id) {
                return Err(Error::InvalidFormat(
                    "invalid manifest chunk sequence".into(),
                ));
            }
        }
        Ok(())
    }
    fn decrypt_chunk(&self, manifest: &Manifest, index: usize) -> Result<Zeroizing<Vec<u8>>> {
        let chunk = manifest.chunks.get(index).ok_or(Error::Authentication)?;
        let object = self.store.object(&chunk.id)?;
        if object.kind != "chunk" {
            return Err(Error::Authentication);
        }
        let aad = chunk_aad(
            self.vault_id(),
            &manifest.file_id,
            &manifest.version_id,
            &chunk.id,
            index,
            chunk.length as usize,
        )?;
        let plaintext = Zeroizing::new(open(&manifest.key, &object.data, &aad)?);
        if plaintext.len() != chunk.length as usize {
            return Err(Error::Authentication);
        }
        Ok(plaintext)
    }
    fn validate_destination(
        &self,
        parent_id: Option<&str>,
        name: &str,
        except: Option<&str>,
    ) -> Result<()> {
        validate_name(name)?;
        if let Some(parent) = parent_id {
            let entry = self.entry(parent)?;
            if entry.kind != EntryKind::Folder || entry.trashed {
                return Err(Error::InvalidInput(
                    "destination is not a live folder".into(),
                ));
            }
        }
        if self.entries.values().any(|resolved| {
            !resolved.entry.trashed
                && resolved.entry.parent_id.as_deref() == parent_id
                && Some(resolved.entry.id.as_str()) != except
                && resolved.entry.name.to_lowercase() == name.to_lowercase()
        }) {
            return Err(Error::AlreadyExists);
        }
        Ok(())
    }
    fn materialize(&mut self, id: &str) -> Result<()> {
        let resolved = self.entries.get(id).ok_or(Error::NotFound)?.clone();
        if !resolved.original {
            self.commit_raw(Operation::Create {
                id: id.into(),
                parent: resolved.entry.parent_id,
                name: resolved.entry.name,
                kind: resolved.entry.kind,
                manifest: resolved.manifest,
                size: resolved.entry.size,
            })?;
        }
        Ok(())
    }
    fn commit(&mut self, operation: Operation) -> Result<()> {
        self.preserve_conflicts()?;
        self.commit_raw(operation)
    }

    fn preserve_conflicts(&mut self) -> Result<()> {
        let aliases: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.original)
            .map(|(id, _)| id.clone())
            .collect();
        for id in aliases {
            self.materialize(&id)?;
        }
        Ok(())
    }

    fn commit_raw(&mut self, operation: Operation) -> Result<()> {
        let id = new_id();
        let commit = Commit {
            id: id.clone(),
            parents: self.heads.clone(),
            device_id: self.local.device_id.clone(),
            modified: now_ms(),
            operation,
        };
        self.store.put_metadata("commit", &id, &commit)?;
        self.local.pending.insert(id);
        self.save_local()?;
        self.rebuild()
    }
    fn save_local(&self) -> Result<()> {
        let plaintext = Zeroizing::new(serde_json::to_vec(&self.local)?);
        let checkpoint_key = Zeroizing::new(derive_key(&self.key, b"local-checkpoint"));
        let ciphertext = seal(&checkpoint_key, &plaintext, self.vault_id().as_bytes())?;
        write_atomic(&self.root.join("local.bin"), &ciphertext)
    }
    fn rebuild(&mut self) -> Result<()> {
        let mut all = BTreeMap::new();
        for (id, kind) in &self.store.kinds {
            if kind != "commit" {
                continue;
            }
            let commit: Commit = self.store.metadata("commit", id)?;
            if let Some(manifest_id) = commit.operation.manifest_id() {
                if !self.has_object(manifest_id) {
                    continue;
                }
                let manifest: Manifest = self.store.metadata("manifest", manifest_id)?;
                self.validate_manifest(&manifest)?;
                let expected_size = match &commit.operation {
                    Operation::Create { size, .. } | Operation::Write { size, .. } => *size,
                    _ => 0,
                };
                if manifest.size != expected_size {
                    return Err(Error::Authentication);
                }
            }
            all.insert(id.clone(), commit);
        }
        let reduction = sync::reduce(&all, &self.local)?;
        if self
            .local
            .known_commits
            .iter()
            .any(|id| !reduction.accepted.contains(id))
        {
            return Err(Error::RollbackDetected);
        }
        self.entries = reduction.entries;
        self.heads = reduction.heads;
        self.commits = all
            .into_iter()
            .filter(|(id, _)| reduction.accepted.contains(id))
            .collect();
        Ok(())
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.encode_utf16().count() > 240
        || name.ends_with([' ', '.'])
        || name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
    {
        return Err(Error::InvalidInput("invalid Windows filename".into()));
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.as_bytes()[3].is_ascii_digit()
        && stem.as_bytes()[3] != b'0')
    {
        return Err(Error::InvalidInput("reserved Windows filename".into()));
    }
    Ok(())
}

fn chunk_aad(
    vault_id: &str,
    file_id: &str,
    version_id: &str,
    object_id: &str,
    index: usize,
    length: usize,
) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "pcloud-vault/v1/chunk",
        vault_id,
        file_id,
        version_id,
        object_id,
        index as u64,
        length as u64,
    ))?)
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u64::MAX as u128) as u64
        })
}

#[cfg(test)]
mod tests;
