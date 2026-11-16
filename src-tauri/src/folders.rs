//! Folder sync and backup engine.
//!
//! A [`FolderJob`] pairs a local folder with a vault folder or a pCloud folder. Each run scans
//! the local tree, lists the remote tree, plans actions against the last known state (the
//! [`JobIndex`]) with the pure [`plan`] function, and executes them with [`run`].
//!
//! Safety rules the executor keeps:
//! - User files are never hard-deleted. Local removals and replaced files move into a dated
//!   recycle folder; remote removals use the vault trash or the pCloud trash.
//! - Downloads land in a temporary file next to the destination and are renamed into place.
//! - A missing or unmounted local folder or remote root aborts the run instead of looking like
//!   "everything was deleted".
//! - Large deletion batches are held for review (see [`exceeds_delete_guard`]).
//!
//! First-run rule: when both sides hold the same path and there is no index entry yet, equal
//! sizes are verified by downloading the remote copy to a temporary file and comparing SHA-256
//! hashes. Identical contents are linked without transferring anything; different contents (or
//! different sizes) become a keep-both conflict for sync jobs, or an upload that replaces the
//! remote copy for backup jobs (the vault and pCloud keep the previous version).

// The commands that drive this engine are wired in separately.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Prefix of temporary download files; the scanner never syncs names with this prefix.
const PARTIAL_PREFIX: &str = ".pcloud-vault-partial-";
const INDEX_SAVE_INTERVAL: usize = 50;
const RECYCLE_RETENTION_DAYS: i64 = 30;
/// Folder under the recycle root for comparing backup files without touching the source.
const VERIFY_DIR: &str = ".verify";
const DAY_MS: u64 = 86_400_000;
/// Error text used when the local folder is not reachable (unplugged drive, network share).
pub const LOCAL_FOLDER_UNAVAILABLE: &str =
    "The local folder is not available. Reconnect the drive or folder and try again.";

// ---------------------------------------------------------------------------------------------
// Jobs and status
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Sync,
    Backup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetLocation {
    Vault,
    Cloud,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderJob {
    /// 16 lowercase hex characters, see [`new_job_id`].
    pub id: String,
    pub kind: JobKind,
    pub local_path: PathBuf,
    pub location: TargetLocation,
    /// Vault entry id, or a pCloud folder as `d<number>`; `None` is the root.
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub mirror_deletions: bool,
    #[serde(default)]
    pub last_run: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Idle,
    Scanning,
    Transferring,
    Paused,
    WaitingForVault,
    WaitingForConnection,
    NeedsReview,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub state: JobState,
    pub message: Option<String>,
    pub pending_deletes: usize,
    pub file_count: usize,
}

impl JobStatus {
    pub fn new(state: JobState) -> Self {
        Self {
            state,
            message: None,
            pending_deletes: 0,
            file_count: 0,
        }
    }

    /// Status after a completed run: held deletions need review, otherwise idle.
    pub fn from_outcome(outcome: &RunOutcome) -> Self {
        Self {
            state: if outcome.pending_deletes > 0 {
                JobState::NeedsReview
            } else {
                JobState::Idle
            },
            message: outcome.errors.first().cloned(),
            pending_deletes: outcome.pending_deletes,
            file_count: outcome.file_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub name: String,
    pub direction: Direction,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub uploaded: usize,
    pub downloaded: usize,
    pub trashed: usize,
    pub recycled: usize,
    pub conflicts: usize,
    /// Deletions held back by the mass-delete guard; non-zero means the user must review.
    pub pending_deletes: usize,
    pub file_count: usize,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Remote targets
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteItem {
    /// `/`-separated path relative to the job root.
    pub rel_path: String,
    pub id: String,
    pub is_folder: bool,
    pub size: u64,
    /// Opaque content version; changes whenever the file contents change.
    pub version: String,
}

pub trait SyncTarget {
    fn list_tree(&mut self) -> Result<Vec<RemoteItem>, String>;
    /// Uploads `local` to `rel`. `existing` is the current remote file at that path, if any.
    fn upload(
        &mut self,
        rel: &str,
        local: &Path,
        existing: Option<&RemoteItem>,
    ) -> Result<RemoteItem, String>;
    /// Writes the remote file to `dest`, which does not exist yet.
    fn download(&mut self, item: &RemoteItem, dest: &Path) -> Result<(), String>;
    /// Creates `rel`; its parent folder already exists.
    fn create_folder(&mut self, rel: &str) -> Result<RemoteItem, String>;
    /// Moves an item to the trash. Folders are only trashed when they are empty.
    fn trash(&mut self, item: &RemoteItem) -> Result<(), String>;
}

pub struct VaultTarget<'a> {
    /// Locked per operation so the app stays responsive during a long run.
    pub vault: &'a std::sync::Mutex<Option<vault_core::Vault>>,
    pub root: Option<String>,
    /// Makes a file's encrypted chunks available locally before it is exported. Called
    /// without the vault lock held.
    pub ensure: &'a dyn Fn(&str) -> Result<(), String>,
    folders: HashMap<String, Option<String>>,
}

impl<'a> VaultTarget<'a> {
    pub fn new(
        vault: &'a std::sync::Mutex<Option<vault_core::Vault>>,
        root: Option<String>,
        ensure: &'a dyn Fn(&str) -> Result<(), String>,
    ) -> Self {
        Self {
            vault,
            root,
            ensure,
            folders: HashMap::new(),
        }
    }

    fn with<T>(
        &self,
        work: impl FnOnce(&mut vault_core::Vault) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .vault
            .lock()
            .map_err(|_| "The vault is unavailable.".to_string())?;
        let vault = guard.as_mut().ok_or("The vault is locked.")?;
        work(vault)
    }

    fn parent_id(&self, rel: &str) -> Result<Option<String>, String> {
        let (dir, _) = split_rel(rel);
        self.folders
            .get(dir)
            .cloned()
            .ok_or_else(|| format!("The vault folder for {rel} is missing."))
    }
}

impl SyncTarget for VaultTarget<'_> {
    fn list_tree(&mut self) -> Result<Vec<RemoteItem>, String> {
        let root = self.root.clone();
        let (items, folders) = self.with(|vault| {
            let mut folders = HashMap::new();
            if let Some(root) = &root {
                let entry = vault.entry(root).map_err(|e| e.to_string())?;
                if entry.kind != vault_core::EntryKind::Folder || entry.trashed {
                    return Err("The vault folder for this job no longer exists.".into());
                }
            }
            folders.insert(String::new(), root.clone());
            let mut items = Vec::new();
            let mut stack = vec![(root.clone(), String::new())];
            while let Some((parent, prefix)) = stack.pop() {
                for entry in vault.list(parent.as_deref(), false) {
                    let rel = join_rel(&prefix, &entry.name);
                    match entry.kind {
                        vault_core::EntryKind::Folder => {
                            folders.insert(rel.clone(), Some(entry.id.clone()));
                            items.push(RemoteItem {
                                rel_path: rel.clone(),
                                id: entry.id.clone(),
                                is_folder: true,
                                size: 0,
                                version: String::new(),
                            });
                            stack.push((Some(entry.id), rel));
                        }
                        vault_core::EntryKind::File => {
                            let version = vault
                                .content_version(&entry.id)
                                .map_err(|e| e.to_string())?;
                            items.push(RemoteItem {
                                rel_path: rel,
                                id: entry.id,
                                is_folder: false,
                                size: entry.size,
                                version,
                            });
                        }
                    }
                }
            }
            Ok((items, folders))
        })?;
        self.folders = folders;
        Ok(items)
    }

    fn upload(
        &mut self,
        rel: &str,
        local: &Path,
        existing: Option<&RemoteItem>,
    ) -> Result<RemoteItem, String> {
        let file = File::open(local).map_err(|e| format!("{rel}: {e}"))?;
        let parent = match existing {
            Some(_) => None,
            None => self.parent_id(rel)?,
        };
        let (_, name) = split_rel(rel);
        self.with(|vault| {
            let entry = match existing {
                Some(item) => vault.replace_file(&item.id, file),
                None => vault.import_file(parent.as_deref(), name, file),
            }
            .map_err(|e| format!("{rel}: {e}"))?;
            let version = vault
                .content_version(&entry.id)
                .map_err(|e| format!("{rel}: {e}"))?;
            Ok(RemoteItem {
                rel_path: rel.into(),
                id: entry.id,
                is_folder: false,
                size: entry.size,
                version,
            })
        })
    }

    fn download(&mut self, item: &RemoteItem, dest: &Path) -> Result<(), String> {
        (self.ensure)(&item.id)?;
        self.with(|vault| vault.export_file(&item.id, dest).map_err(|e| e.to_string()))
    }

    fn create_folder(&mut self, rel: &str) -> Result<RemoteItem, String> {
        let parent = self.parent_id(rel)?;
        let (_, name) = split_rel(rel);
        let entry = self.with(|vault| {
            vault
                .create_folder(parent.as_deref(), name)
                .map_err(|e| format!("{rel}: {e}"))
        })?;
        self.folders.insert(rel.into(), Some(entry.id.clone()));
        Ok(RemoteItem {
            rel_path: rel.into(),
            id: entry.id,
            is_folder: true,
            size: 0,
            version: String::new(),
        })
    }

    fn trash(&mut self, item: &RemoteItem) -> Result<(), String> {
        self.with(|vault| {
            // Trashing a folder trashes its contents; only do it when it is empty right now.
            if item.is_folder && !vault.list(Some(&item.id), false).is_empty() {
                return Err(format!("{} is not empty; kept.", item.rel_path));
            }
            vault.set_trashed(&item.id, true).map_err(|e| e.to_string())
        })?;
        if item.is_folder {
            self.folders.remove(&item.rel_path);
        }
        Ok(())
    }
}

pub struct CloudTarget<'a> {
    pub client: &'a pcloud_api::Client,
    pub root: u64,
    folders: HashMap<String, u64>,
}

impl<'a> CloudTarget<'a> {
    pub fn new(client: &'a pcloud_api::Client, root: u64) -> Self {
        Self {
            client,
            root,
            folders: HashMap::new(),
        }
    }

    fn parent_id(&self, rel: &str) -> Result<u64, String> {
        let (dir, _) = split_rel(rel);
        self.folders
            .get(dir)
            .copied()
            .ok_or_else(|| format!("The pCloud folder for {rel} is missing."))
    }
}

fn cloud_item(rel: String, metadata: &pcloud_api::Metadata) -> RemoteItem {
    RemoteItem {
        rel_path: rel,
        id: format!(
            "{}{}",
            if metadata.is_folder { "d" } else { "f" },
            metadata.id
        ),
        is_folder: metadata.is_folder,
        size: metadata.size,
        version: if metadata.is_folder {
            String::new()
        } else {
            match metadata.hash {
                Some(hash) => format!("{}:{}:{hash}", metadata.size, metadata.modified),
                None => format!("{}:{}", metadata.size, metadata.modified),
            }
        },
    }
}

fn cloud_numeric_id(item: &RemoteItem) -> Result<u64, String> {
    item.id
        .get(1..)
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("Invalid pCloud id for {}.", item.rel_path))
}

/// Parses a job's `folder_id` (`d<number>`, or `None` for the root) into a pCloud folder id.
pub fn cloud_folder_id(folder_id: Option<&str>) -> Result<u64, String> {
    match folder_id {
        None => Ok(0),
        Some(value) => value
            .strip_prefix('d')
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| "Invalid pCloud folder.".to_string()),
    }
}

impl SyncTarget for CloudTarget<'_> {
    fn list_tree(&mut self) -> Result<Vec<RemoteItem>, String> {
        let listing = self
            .client
            .list_tree(self.root)
            .map_err(|e| e.to_string())?;
        let mut paths: HashMap<u64, String> = HashMap::from([(self.root, String::new())]);
        self.folders = HashMap::from([(String::new(), self.root)]);
        let mut items = Vec::with_capacity(listing.len());
        for metadata in &listing {
            let Some(parent) = paths.get(&metadata.parent_folder_id).cloned() else {
                continue;
            };
            let rel = join_rel(&parent, &metadata.name);
            if metadata.is_folder {
                paths.insert(metadata.id, rel.clone());
                self.folders.insert(rel.clone(), metadata.id);
            }
            items.push(cloud_item(rel, metadata));
        }
        Ok(items)
    }

    fn upload(
        &mut self,
        rel: &str,
        local: &Path,
        _existing: Option<&RemoteItem>,
    ) -> Result<RemoteItem, String> {
        // pCloud replaces a same-named file in place (keeping its revision history).
        let parent = self.parent_id(rel)?;
        let (_, name) = split_rel(rel);
        let file = File::open(local).map_err(|e| format!("{rel}: {e}"))?;
        let length = file.metadata().map_err(|e| format!("{rel}: {e}"))?.len();
        let metadata = self
            .client
            .upload_reader(parent, name, file.take(length), length, false)
            .map_err(|e| format!("{rel}: {e}"))?;
        Ok(cloud_item(rel.into(), &metadata))
    }

    fn download(&mut self, item: &RemoteItem, dest: &Path) -> Result<(), String> {
        let id = cloud_numeric_id(item)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)
            .map_err(|e| e.to_string())?;
        let result = self
            .client
            .download_to(id, &mut file)
            .map_err(|e| e.to_string())
            .and_then(|_| file.sync_all().map_err(|e| e.to_string()));
        if result.is_err() {
            drop(file);
            let _ = fs::remove_file(dest);
        }
        result
    }

    fn create_folder(&mut self, rel: &str) -> Result<RemoteItem, String> {
        let parent = self.parent_id(rel)?;
        let (_, name) = split_rel(rel);
        let metadata = self
            .client
            .create_folder(parent, name)
            .map_err(|e| format!("{rel}: {e}"))?;
        self.folders.insert(rel.into(), metadata.id);
        Ok(cloud_item(rel.into(), &metadata))
    }

    fn trash(&mut self, item: &RemoteItem) -> Result<(), String> {
        let id = cloud_numeric_id(item)?;
        if item.is_folder {
            // `delete_folder` is recursive; re-check emptiness right before calling it.
            let contents = self.client.list_folder(id).map_err(|e| e.to_string())?;
            if !contents.is_empty() {
                return Err(format!("{} is not empty; kept.", item.rel_path));
            }
            self.client.delete_folder(id).map_err(|e| e.to_string())?;
            self.folders.remove(&item.rel_path);
        } else {
            self.client.delete_file(id).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    pub local_size: u64,
    pub local_mtime_ms: u64,
    /// Hex SHA-256 of the local contents last synced; empty means "unknown, re-check".
    pub local_sha256: String,
    pub remote_id: String,
    pub remote_version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobIndex {
    #[serde(default)]
    pub files: BTreeMap<String, IndexEntry>,
    #[serde(default)]
    pub folders: BTreeSet<String>,
}

pub trait IndexStore {
    fn load(&self, job_id: &str) -> Result<JobIndex, String>;
    fn save(&self, job_id: &str, index: &JobIndex) -> Result<(), String>;
    fn remove(&self, job_id: &str) -> Result<(), String>;
}

fn check_job_id(job_id: &str) -> Result<(), String> {
    if is_job_id(job_id) {
        Ok(())
    } else {
        Err("Invalid folder job id.".into())
    }
}

fn is_job_id(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Plain JSON index files, `<dir>/<job id>.json`, for jobs whose names are not secret.
pub struct JsonIndexStore {
    pub dir: PathBuf,
}

impl JsonIndexStore {
    fn path(&self, job_id: &str) -> Result<PathBuf, String> {
        check_job_id(job_id)?;
        Ok(self.dir.join(format!("{job_id}.json")))
    }
}

impl IndexStore for JsonIndexStore {
    fn load(&self, job_id: &str) -> Result<JobIndex, String> {
        let path = self.path(job_id)?;
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("The folder sync index is damaged: {e}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(JobIndex::default()),
            Err(error) => Err(format!("Unable to read the folder sync index: {error}")),
        }
    }

    fn save(&self, job_id: &str, index: &JobIndex) -> Result<(), String> {
        let path = self.path(job_id)?;
        let bytes = serde_json::to_vec(index).map_err(|e| e.to_string())?;
        write_atomic(&path, &bytes).map_err(|e| format!("Unable to save the sync index: {e}"))
    }

    fn remove(&self, job_id: &str) -> Result<(), String> {
        let path = self.path(job_id)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("index path has no parent"))?;
    fs::create_dir_all(dir)?;
    let temp = path.with_extension("json.tmp");
    {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&temp, path)?;
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    Ok(())
}

/// Index stored encrypted inside the vault's per-device side data (blob `sync-<job id>`).
pub struct VaultIndexStore<'a> {
    pub vault: &'a std::sync::Mutex<Option<vault_core::Vault>>,
}

impl VaultIndexStore<'_> {
    fn with<T>(
        &self,
        work: impl FnOnce(&vault_core::Vault) -> Result<T, String>,
    ) -> Result<T, String> {
        let guard = self
            .vault
            .lock()
            .map_err(|_| "The vault is unavailable.".to_string())?;
        work(guard.as_ref().ok_or("The vault is locked.")?)
    }
}

impl IndexStore for VaultIndexStore<'_> {
    fn load(&self, job_id: &str) -> Result<JobIndex, String> {
        check_job_id(job_id)?;
        match self.with(|vault| {
            vault
                .load_local_blob(&format!("sync-{job_id}"))
                .map_err(|e| e.to_string())
        })? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("The folder sync index is damaged: {e}")),
            None => Ok(JobIndex::default()),
        }
    }

    fn save(&self, job_id: &str, index: &JobIndex) -> Result<(), String> {
        check_job_id(job_id)?;
        let bytes = zeroize::Zeroizing::new(serde_json::to_vec(index).map_err(|e| e.to_string())?);
        self.with(|vault| {
            vault
                .store_local_blob(&format!("sync-{job_id}"), &bytes)
                .map_err(|e| e.to_string())
        })
    }

    fn remove(&self, job_id: &str) -> Result<(), String> {
        check_job_id(job_id)?;
        self.with(|vault| {
            vault
                .remove_local_blob(&format!("sync-{job_id}"))
                .map_err(|e| e.to_string())
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Local scan
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub size: u64,
    pub mtime_ms: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default)]
pub struct LocalScan {
    pub files: BTreeMap<String, LocalFile>,
    pub folders: BTreeSet<String>,
    /// Paths that exist but cannot be synced (symlinks, unreadable files or folders). They
    /// and everything below them are left alone on both sides.
    pub skipped: BTreeSet<String>,
    pub errors: Vec<String>,
}

impl LocalScan {
    pub fn is_skipped(&self, rel: &str) -> bool {
        if self.skipped.is_empty() {
            return false;
        }
        let mut end = rel.len();
        loop {
            if self.skipped.contains(&rel[..end]) {
                return true;
            }
            match rel[..end].rfind('/') {
                Some(slash) => end = slash,
                None => return false,
            }
        }
    }
}

/// Names that are never synced: editor/OS temp files, partial downloads, and names the vault
/// cannot store.
pub fn is_ignored_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with("~$")
        || lower.ends_with(".tmp")
        || lower.ends_with(".part")
        || name.starts_with(".~lock")
        || name == ".DS_Store"
        || lower == "desktop.ini"
        || lower == "thumbs.db"
        || name.starts_with(".pcloud-vault")
        || vault_core::validate_name(name).is_err()
}

fn is_ignored_path(rel: &str) -> bool {
    rel.split('/').any(is_ignored_name)
}

/// Walks `root` without following symlinks. Hashes are reused from `index` when size and
/// modification time are unchanged.
pub fn scan_local(root: &Path, index: &JobIndex) -> Result<LocalScan, String> {
    let mut scan = LocalScan::default();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel_dir)) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if rel_dir.is_empty() => {
                return Err(format!("{LOCAL_FOLDER_UNAVAILABLE} ({error})"))
            }
            Err(error) => {
                scan.errors.push(format!("Skipped {rel_dir}: {error}"));
                scan.skipped.insert(rel_dir);
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if rel_dir.is_empty() => {
                    return Err(format!("{LOCAL_FOLDER_UNAVAILABLE} ({error})"))
                }
                Err(error) => {
                    scan.errors.push(format!("Skipped {rel_dir}: {error}"));
                    scan.skipped.insert(rel_dir.clone());
                    break;
                }
            };
            let Ok(name) = entry.file_name().into_string() else {
                let place: &str = if rel_dir.is_empty() { "/" } else { &rel_dir };
                scan.errors.push(format!(
                    "Skipped a name in {place} that is not valid Unicode."
                ));
                continue;
            };
            if name.starts_with(PARTIAL_PREFIX) {
                remove_stale_partial(&entry.path());
                continue;
            }
            if is_ignored_name(&name) {
                continue;
            }
            let rel = join_rel(&rel_dir, &name);
            let Ok(file_type) = entry.file_type() else {
                scan.skipped.insert(rel);
                continue;
            };
            if file_type.is_symlink() {
                scan.skipped.insert(rel);
            } else if file_type.is_dir() {
                scan.folders.insert(rel.clone());
                stack.push((entry.path(), rel));
            } else if file_type.is_file() {
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        scan.errors.push(format!("Skipped {rel}: {error}"));
                        scan.skipped.insert(rel);
                        continue;
                    }
                };
                let size = metadata.len();
                let mtime_ms = mtime_ms(&metadata);
                let known = index.files.get(&rel).filter(|known| {
                    known.local_size == size
                        && known.local_mtime_ms == mtime_ms
                        && !known.local_sha256.is_empty()
                });
                let sha256 = match known {
                    Some(known) => known.local_sha256.clone(),
                    None => match hash_file(&entry.path()) {
                        Ok(hash) => hash,
                        Err(error) => {
                            scan.errors.push(format!("Skipped {rel}: {error}"));
                            scan.skipped.insert(rel);
                            continue;
                        }
                    },
                };
                scan.files.insert(
                    rel,
                    LocalFile {
                        size,
                        mtime_ms,
                        sha256,
                    },
                );
            } else {
                scan.skipped.insert(rel);
            }
        }
    }
    Ok(scan)
}

/// Removes our own temporary download files left behind by a crash, once they are a day old.
fn remove_stale_partial(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    let old = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age.as_secs() > 24 * 60 * 60);
    if metadata.is_file() && old {
        let _ = fs::remove_file(path);
    }
}

fn mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 15)] as char);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    CreateRemoteFolder {
        rel: String,
    },
    CreateLocalFolder {
        rel: String,
    },
    Upload {
        rel: String,
        existing: Option<RemoteItem>,
    },
    Download {
        item: RemoteItem,
    },
    /// Both sides changed: the local file is renamed to a conflict copy and uploaded, and the
    /// remote version is downloaded to the original name.
    ConflictKeepBoth {
        rel: String,
        item: RemoteItem,
    },
    /// First run with the same path and size on both sides: compare contents before linking.
    VerifySame {
        rel: String,
        item: RemoteItem,
    },
    /// Moves a remote file, or an empty remote folder, to the trash.
    TrashRemote {
        item: RemoteItem,
    },
    /// Moves a local file into the recycle folder.
    RecycleLocal {
        rel: String,
    },
    /// Removes a local folder only if it is empty.
    RemoveLocalFolder {
        rel: String,
    },
    ForgetIndex {
        rel: String,
    },
}

impl Action {
    fn is_deletion(&self) -> bool {
        matches!(
            self,
            Self::TrashRemote { .. } | Self::RecycleLocal { .. } | Self::RemoveLocalFolder { .. }
        )
    }

    fn is_transfer(&self) -> bool {
        matches!(
            self,
            Self::Upload { .. }
                | Self::Download { .. }
                | Self::ConflictKeepBoth { .. }
                | Self::VerifySame { .. }
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
    /// Paths that were left alone, with the reason.
    pub notes: Vec<String>,
}

/// Number of file deletions in a plan (remote trash + local recycle), used by the guard.
pub fn deletion_count(plan: &Plan) -> usize {
    plan.actions
        .iter()
        .filter(|action| match action {
            Action::TrashRemote { item } => !item.is_folder,
            Action::RecycleLocal { .. } => true,
            _ => false,
        })
        .count()
}

/// True when a deletion batch is large enough to need the user's confirmation.
pub fn exceeds_delete_guard(deletions: usize, indexed_files: usize) -> bool {
    deletions > 50 || (deletions > 5 && deletions * 4 > indexed_files)
}

/// Computes the actions that bring both sides in line. Order: folder creation (parents
/// first), transfers, file deletions, index cleanup, folder removal (children first).
pub fn plan(
    kind: JobKind,
    mirror_deletions: bool,
    local: &LocalScan,
    index: &JobIndex,
    remote: &[RemoteItem],
) -> Plan {
    let mut remote_files: BTreeMap<&str, &RemoteItem> = BTreeMap::new();
    let mut remote_folders: BTreeMap<&str, &RemoteItem> = BTreeMap::new();
    for item in remote {
        if is_ignored_path(&item.rel_path) || local.is_skipped(&item.rel_path) {
            continue;
        }
        if item.is_folder {
            remote_folders.insert(item.rel_path.as_str(), item);
        } else {
            remote_files.insert(item.rel_path.as_str(), item);
        }
    }

    // Names that differ only by letter case can't coexist in the vault or on Windows/macOS
    // disks; leave them alone instead of uploading one and trashing the other.
    let mut by_lowercase: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for rel in local
        .files
        .keys()
        .chain(&local.folders)
        .chain(index.files.keys())
        .chain(&index.folders)
        .map(String::as_str)
        .chain(remote_files.keys().copied())
        .chain(remote_folders.keys().copied())
    {
        by_lowercase
            .entry(rel.to_lowercase())
            .or_default()
            .insert(rel);
    }
    let case_clashes: BTreeSet<&str> = by_lowercase
        .into_values()
        .filter(|group| group.len() > 1)
        .flatten()
        .collect();

    let mut notes: Vec<String> = case_clashes
        .iter()
        .map(|rel| format!("{rel} differs from another name only by letter case; left alone."))
        .collect();
    let mut creates = Vec::new();
    let mut transfers = Vec::new();
    let mut deletions = Vec::new();
    let mut forgets = Vec::new();
    let mut folder_removals = Vec::new();

    let folder_paths: BTreeSet<&str> = local
        .folders
        .iter()
        .map(String::as_str)
        .chain(index.folders.iter().map(String::as_str))
        .chain(remote_folders.keys().copied())
        .collect();
    for rel in folder_paths {
        if local.is_skipped(rel) || is_ignored_path(rel) || case_clashes.contains(rel) {
            continue;
        }
        let on_local = local.folders.contains(rel);
        let on_remote = remote_folders.get(rel).copied();
        let indexed = index.folders.contains(rel);
        if (on_local && remote_files.contains_key(rel))
            || (on_remote.is_some() && local.files.contains_key(rel))
        {
            notes.push(format!(
                "{rel} is a file on one side and a folder on the other; left alone."
            ));
            continue;
        }
        let rel = rel.to_string();
        match (kind, on_local, on_remote, indexed) {
            (_, true, Some(_), _) => {}
            (JobKind::Sync, true, None, false) | (JobKind::Backup, true, None, _) => {
                creates.push(Action::CreateRemoteFolder { rel })
            }
            (JobKind::Sync, false, Some(_), false) => {
                creates.push(Action::CreateLocalFolder { rel })
            }
            (JobKind::Sync, true, None, true) => {
                folder_removals.push(Action::RemoveLocalFolder { rel })
            }
            (JobKind::Sync, false, Some(item), true) => {
                folder_removals.push(Action::TrashRemote { item: item.clone() })
            }
            (JobKind::Backup, false, Some(item), true) if mirror_deletions => {
                folder_removals.push(Action::TrashRemote { item: item.clone() })
            }
            (_, false, _, true) => forgets.push(Action::ForgetIndex { rel }),
            (_, false, _, false) => {}
        }
    }

    let file_paths: BTreeSet<&str> = local
        .files
        .keys()
        .map(String::as_str)
        .chain(index.files.keys().map(String::as_str))
        .chain(remote_files.keys().copied())
        .collect();
    for rel in file_paths {
        if local.is_skipped(rel) || is_ignored_path(rel) || case_clashes.contains(rel) {
            continue;
        }
        if local.folders.contains(rel) || remote_folders.contains_key(rel) {
            if !(local.files.contains_key(rel) || remote_files.contains_key(rel)) {
                forgets.push(Action::ForgetIndex { rel: rel.into() });
            } else {
                notes.push(format!(
                    "{rel} is a file on one side and a folder on the other; left alone."
                ));
            }
            continue;
        }
        let on_local = local.files.get(rel);
        let indexed = index.files.get(rel);
        let on_remote = remote_files.get(rel).copied();
        let local_changed = match (on_local, indexed) {
            (Some(file), Some(entry)) => file.sha256 != entry.local_sha256,
            _ => true,
        };
        let remote_changed = match (on_remote, indexed) {
            (Some(item), Some(entry)) => {
                item.version != entry.remote_version || item.id != entry.remote_id
            }
            _ => true,
        };
        let rel = rel.to_string();
        match (kind, on_local, indexed, on_remote) {
            // Present on both sides with history.
            (JobKind::Sync, Some(_), Some(_), Some(item)) => {
                match (local_changed, remote_changed) {
                    (false, false) => {}
                    (true, false) => transfers.push(Action::Upload {
                        rel,
                        existing: Some(item.clone()),
                    }),
                    (false, true) => transfers.push(Action::Download { item: item.clone() }),
                    (true, true) => transfers.push(Action::ConflictKeepBoth {
                        rel,
                        item: item.clone(),
                    }),
                }
            }
            (JobKind::Backup, Some(_), Some(_), Some(item)) => {
                // A remote-only edit is kept; the local copy is the source of truth for edits.
                if local_changed {
                    transfers.push(Action::Upload {
                        rel,
                        existing: Some(item.clone()),
                    });
                }
            }
            // Present on both sides without history (first run, or created on both).
            (_, Some(file), None, Some(item)) => {
                if file.size == item.size {
                    transfers.push(Action::VerifySame {
                        rel,
                        item: item.clone(),
                    });
                } else if kind == JobKind::Sync {
                    transfers.push(Action::ConflictKeepBoth {
                        rel,
                        item: item.clone(),
                    });
                } else {
                    transfers.push(Action::Upload {
                        rel,
                        existing: Some(item.clone()),
                    });
                }
            }
            // Local only.
            (JobKind::Sync, Some(_), Some(_), None) if !local_changed => {
                deletions.push(Action::RecycleLocal { rel })
            }
            (_, Some(_), _, None) => transfers.push(Action::Upload {
                rel,
                existing: None,
            }),
            // Remote only.
            (JobKind::Sync, None, Some(_), Some(item)) if !remote_changed => {
                deletions.push(Action::TrashRemote { item: item.clone() })
            }
            (JobKind::Sync, None, _, Some(item)) => {
                transfers.push(Action::Download { item: item.clone() })
            }
            (JobKind::Backup, None, Some(_), Some(item)) => {
                if mirror_deletions && !remote_changed {
                    deletions.push(Action::TrashRemote { item: item.clone() });
                } else {
                    forgets.push(Action::ForgetIndex { rel });
                }
            }
            (JobKind::Backup, None, None, Some(_)) => {}
            // Gone on both sides.
            (_, None, Some(_), None) => forgets.push(Action::ForgetIndex { rel }),
            (_, None, None, None) => {}
        }
    }

    folder_removals.reverse();
    let mut actions = creates;
    actions.extend(transfers);
    actions.extend(deletions);
    actions.extend(forgets);
    actions.extend(folder_removals);
    Plan { actions, notes }
}

/// Turns deletions into re-copies: remote trash becomes a download, local recycle becomes an
/// upload, and folder removals become folder creation on the side where it is missing.
pub fn plan_restore(plan: &Plan) -> Plan {
    let mut creates = Vec::new();
    let mut others = Vec::new();
    for action in &plan.actions {
        match action {
            Action::TrashRemote { item } if item.is_folder => {
                creates.push(Action::CreateLocalFolder {
                    rel: item.rel_path.clone(),
                })
            }
            Action::TrashRemote { item } => others.push(Action::Download { item: item.clone() }),
            Action::RecycleLocal { rel } => others.push(Action::Upload {
                rel: rel.clone(),
                existing: None,
            }),
            Action::RemoveLocalFolder { rel } => {
                creates.push(Action::CreateRemoteFolder { rel: rel.clone() })
            }
            action @ (Action::CreateLocalFolder { .. } | Action::CreateRemoteFolder { .. }) => {
                creates.push(action.clone())
            }
            action => others.push(action.clone()),
        }
    }
    creates.sort_by(|a, b| action_rel(a).cmp(action_rel(b)));
    creates.extend(others);
    Plan {
        actions: creates,
        notes: plan.notes.clone(),
    }
}

fn action_rel(action: &Action) -> &str {
    match action {
        Action::CreateRemoteFolder { rel }
        | Action::CreateLocalFolder { rel }
        | Action::Upload { rel, .. }
        | Action::ConflictKeepBoth { rel, .. }
        | Action::VerifySame { rel, .. }
        | Action::RecycleLocal { rel }
        | Action::RemoveLocalFolder { rel }
        | Action::ForgetIndex { rel } => rel,
        Action::Download { item } | Action::TrashRemote { item } => &item.rel_path,
    }
}

// ---------------------------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------------------------

/// Runs one sync or backup pass. Large deletion batches are held unless `allow_deletes`.
pub fn run(
    job: &FolderJob,
    target: &mut dyn SyncTarget,
    store: &dyn IndexStore,
    recycle_root: &Path,
    allow_deletes: bool,
    progress: &mut dyn FnMut(Progress),
    now_ms: u64,
) -> Result<RunOutcome, String> {
    execute(
        job,
        target,
        store,
        recycle_root,
        Mode::Normal { allow_deletes },
        progress,
        now_ms,
    )
}

/// Runs a pass in which every planned deletion is replaced by copying the file back.
pub fn run_restore(
    job: &FolderJob,
    target: &mut dyn SyncTarget,
    store: &dyn IndexStore,
    recycle_root: &Path,
    progress: &mut dyn FnMut(Progress),
    now_ms: u64,
) -> Result<RunOutcome, String> {
    execute(
        job,
        target,
        store,
        recycle_root,
        Mode::Restore,
        progress,
        now_ms,
    )
}

#[derive(Clone, Copy)]
enum Mode {
    Normal { allow_deletes: bool },
    Restore,
}

fn ensure_local_root(root: &Path) -> Result<(), String> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        _ => Err(LOCAL_FOLDER_UNAVAILABLE.into()),
    }
}

fn execute(
    job: &FolderJob,
    target: &mut dyn SyncTarget,
    store: &dyn IndexStore,
    recycle_root: &Path,
    mode: Mode,
    progress: &mut dyn FnMut(Progress),
    now_ms: u64,
) -> Result<RunOutcome, String> {
    check_job_id(&job.id)?;
    ensure_local_root(&job.local_path)?;
    let mut index = store.load(&job.id)?;
    let scan = scan_local(&job.local_path, &index)?;
    // An unmount during the scan must not look like a folder that was emptied.
    ensure_local_root(&job.local_path)?;
    let remote = target.list_tree()?;

    let mut plan = plan(job.kind, job.mirror_deletions, &scan, &index, &remote);
    let mut outcome = RunOutcome {
        errors: scan.errors.clone(),
        ..RunOutcome::default()
    };
    outcome.errors.extend(plan.notes.iter().cloned());
    match mode {
        Mode::Restore => plan = plan_restore(&plan),
        Mode::Normal { allow_deletes } => {
            let deletions = deletion_count(&plan);
            // An empty folder (e.g. an unmounted mount point) never deletes without review.
            let local_empty = scan.files.is_empty() && scan.folders.is_empty();
            if !allow_deletes
                && (exceeds_delete_guard(deletions, index.files.len())
                    || (local_empty && deletions > 0))
            {
                plan.actions.retain(|action| !action.is_deletion());
                outcome.pending_deletes = deletions;
            }
        }
    }

    // Unchanged contents with a new timestamp: remember the timestamp to skip re-hashing.
    for (rel, file) in &scan.files {
        if let Some(entry) = index.files.get_mut(rel) {
            if entry.local_sha256 == file.sha256 {
                entry.local_size = file.size;
                entry.local_mtime_ms = file.mtime_ms;
            }
        }
    }
    let remote_folders: BTreeSet<String> = remote
        .iter()
        .filter(|item| item.is_folder)
        .map(|item| item.rel_path.clone())
        .collect();
    for folder in &scan.folders {
        if remote_folders.contains(folder) {
            index.folders.insert(folder.clone());
        }
    }

    let total = plan.actions.iter().filter(|a| a.is_transfer()).count();
    let mut context = Context {
        job,
        root: &job.local_path,
        target,
        recycle_root,
        scan: &scan,
        now_ms,
        remote_all: remote.iter().map(|item| item.rel_path.clone()).collect(),
        remote_folders,
        index,
        outcome,
    };
    let mut done = 0;
    for (number, action) in plan.actions.into_iter().enumerate() {
        if action.is_transfer() {
            let (name, direction) = match &action {
                Action::Upload { rel, .. } => (rel.clone(), Direction::Upload),
                Action::Download { item }
                | Action::ConflictKeepBoth { item, .. }
                | Action::VerifySame { item, .. } => (item.rel_path.clone(), Direction::Download),
                _ => unreachable!(),
            };
            progress(Progress {
                name,
                direction,
                done,
                total,
            });
            done += 1;
        }
        if let Err(error) = context.apply(action) {
            context.outcome.errors.push(error);
        }
        if (number + 1) % INDEX_SAVE_INTERVAL == 0 {
            if let Err(error) = store.save(&job.id, &context.index) {
                context.outcome.errors.push(error);
            }
        }
    }
    if let Err(error) = store.save(&job.id, &context.index) {
        context.outcome.errors.push(error);
    }
    context.outcome.file_count = context.index.files.len();
    Ok(context.outcome)
}

struct Context<'a> {
    job: &'a FolderJob,
    root: &'a Path,
    target: &'a mut dyn SyncTarget,
    recycle_root: &'a Path,
    scan: &'a LocalScan,
    now_ms: u64,
    /// Every remote path known to exist, including ignored names (for folder emptiness).
    remote_all: BTreeSet<String>,
    remote_folders: BTreeSet<String>,
    index: JobIndex,
    outcome: RunOutcome,
}

/// A downloaded temporary file and the fingerprint it had when the download finished.
struct Downloaded {
    path: PathBuf,
    file: LocalFile,
}

fn fingerprint(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    metadata
        .is_file()
        .then(|| (metadata.len(), mtime_ms(&metadata)))
}

fn exists_no_follow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

impl Context<'_> {
    fn local(&self, rel: &str) -> PathBuf {
        local_path(self.root, rel)
    }

    fn unchanged_since_scan(&self, rel: &str) -> bool {
        self.scan
            .files
            .get(rel)
            .is_some_and(|file| fingerprint(&self.local(rel)) == Some((file.size, file.mtime_ms)))
    }

    fn apply(&mut self, action: Action) -> Result<(), String> {
        match action {
            Action::CreateRemoteFolder { rel } => self.ensure_remote_dirs(&rel),
            Action::CreateLocalFolder { rel } => {
                fs::create_dir_all(self.local(&rel)).map_err(|e| format!("{rel}: {e}"))?;
                self.index.folders.insert(rel);
                Ok(())
            }
            Action::Upload { rel, existing } => self.upload(&rel, existing),
            Action::Download { item } => self.download(&item),
            Action::ConflictKeepBoth { rel, item } => self.conflict(&rel, &item, None),
            Action::VerifySame { rel, item } => self.verify_same(&rel, &item),
            Action::TrashRemote { item } => self.trash_remote(&item),
            Action::RecycleLocal { rel } => {
                if !self.unchanged_since_scan(&rel) {
                    // Edited since the scan; the next run uploads it again.
                    return Ok(());
                }
                self.recycle(&rel)?;
                self.index.files.remove(&rel);
                self.outcome.recycled += 1;
                Ok(())
            }
            Action::RemoveLocalFolder { rel } => {
                match fs::remove_dir(self.local(&rel)) {
                    Ok(()) => {
                        self.index.folders.remove(&rel);
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        self.index.folders.remove(&rel);
                    }
                    // Not empty (or in use): keep it.
                    Err(_) => {}
                }
                Ok(())
            }
            Action::ForgetIndex { rel } => {
                self.index.files.remove(&rel);
                self.index.folders.remove(&rel);
                Ok(())
            }
        }
    }

    /// Creates `dir_rel` and its missing ancestors on the remote side.
    fn ensure_remote_dirs(&mut self, dir_rel: &str) -> Result<(), String> {
        if dir_rel.is_empty() {
            return Ok(());
        }
        let mut prefix = String::new();
        for component in dir_rel.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if !self.remote_folders.contains(&prefix) {
                if self.remote_all.contains(&prefix) {
                    return Err(format!("{prefix} is a file on the remote side."));
                }
                self.target.create_folder(&prefix)?;
                self.remote_folders.insert(prefix.clone());
                self.remote_all.insert(prefix.clone());
            }
            self.index.folders.insert(prefix.clone());
        }
        Ok(())
    }

    fn upload(&mut self, rel: &str, existing: Option<RemoteItem>) -> Result<(), String> {
        let Some(scanned) = self.scan.files.get(rel) else {
            return Err(format!("{rel} is no longer available locally."));
        };
        if !self.unchanged_since_scan(rel) {
            return Err(format!(
                "{rel} changed during the scan; it will be uploaded next run."
            ));
        }
        self.ensure_remote_dirs(split_rel(rel).0)?;
        let path = self.local(rel);
        let item = self.target.upload(rel, &path, existing.as_ref())?;
        self.record_upload(rel, &item, scanned.clone());
        Ok(())
    }

    /// Records an upload. If the file changed while it was read, the hash is left unknown so
    /// the next run uploads it again.
    fn record_upload(&mut self, rel: &str, item: &RemoteItem, scanned: LocalFile) {
        let path = self.local(rel);
        let sha256 = if fingerprint(&path) == Some((scanned.size, scanned.mtime_ms)) {
            scanned.sha256
        } else {
            self.outcome.errors.push(format!(
                "{rel} changed while uploading; it will be uploaded again next run."
            ));
            String::new()
        };
        self.index.files.insert(
            rel.into(),
            IndexEntry {
                local_size: scanned.size,
                local_mtime_ms: scanned.mtime_ms,
                local_sha256: sha256,
                remote_id: item.id.clone(),
                remote_version: item.version.clone(),
            },
        );
        self.remote_all.insert(rel.into());
        self.outcome.uploaded += 1;
    }

    /// Downloads into a temporary file in `dir` (default: the destination folder) and
    /// fingerprints the temporary file, so later local edits can't be mistaken for it.
    fn download_temp(
        &mut self,
        item: &RemoteItem,
        dir: Option<&Path>,
    ) -> Result<Downloaded, String> {
        let rel = item.rel_path.as_str();
        let destination = self.local(rel);
        let dir = match dir {
            Some(dir) => dir.to_path_buf(),
            None => destination
                .parent()
                .ok_or_else(|| format!("{rel}: invalid path"))?
                .to_path_buf(),
        };
        fs::create_dir_all(&dir).map_err(|e| format!("{rel}: {e}"))?;
        let path = dir.join(format!("{PARTIAL_PREFIX}{}", random_hex(8)));
        let described = self.target.download(item, &path).and_then(|()| {
            let (size, mtime_ms) =
                fingerprint(&path).ok_or_else(|| "download is missing".to_string())?;
            let sha256 = hash_file(&path).map_err(|e| e.to_string())?;
            Ok(LocalFile {
                size,
                mtime_ms,
                sha256,
            })
        });
        match described {
            Ok(file) => Ok(Downloaded { path, file }),
            Err(error) => {
                let _ = fs::remove_file(&path);
                Err(format!("{rel}: {error}"))
            }
        }
    }

    /// Records a download that was renamed into place (a rename keeps size and timestamp).
    fn record_download(&mut self, item: &RemoteItem, file: LocalFile) {
        self.index.files.insert(
            item.rel_path.clone(),
            IndexEntry {
                local_size: file.size,
                local_mtime_ms: file.mtime_ms,
                local_sha256: file.sha256,
                remote_id: item.id.clone(),
                remote_version: item.version.clone(),
            },
        );
        self.outcome.downloaded += 1;
    }

    /// Records that the local and remote files have identical contents.
    fn link(&mut self, rel: &str, item: &RemoteItem, scanned: LocalFile) {
        self.index.files.insert(
            rel.into(),
            IndexEntry {
                local_size: scanned.size,
                local_mtime_ms: scanned.mtime_ms,
                local_sha256: scanned.sha256,
                remote_id: item.id.clone(),
                remote_version: item.version.clone(),
            },
        );
    }

    fn download(&mut self, item: &RemoteItem) -> Result<(), String> {
        let rel = item.rel_path.as_str();
        let path = self.local(rel);
        let temp = self.download_temp(item, None)?;
        let placed = (|| {
            if exists_no_follow(&path) {
                if !self.unchanged_since_scan(rel) {
                    return Err(format!(
                        "{rel} changed locally during the sync; it will be checked again next run."
                    ));
                }
                self.recycle(rel)?;
            }
            fs::rename(&temp.path, &path).map_err(|e| format!("{rel}: {e}"))
        })();
        if placed.is_err() {
            let _ = fs::remove_file(&temp.path);
        }
        placed?;
        self.record_download(item, temp.file);
        Ok(())
    }

    fn conflict(
        &mut self,
        rel: &str,
        item: &RemoteItem,
        downloaded: Option<Downloaded>,
    ) -> Result<(), String> {
        if !self.unchanged_since_scan(rel) {
            if let Some(temp) = downloaded {
                let _ = fs::remove_file(temp.path);
            }
            return Err(format!(
                "{rel} changed locally during the sync; it will be checked again next run."
            ));
        }
        let temp = match downloaded {
            Some(temp) => temp,
            None => self.download_temp(item, None)?,
        };
        let scanned = self.scan.files[rel].clone();
        if temp.file.sha256 == scanned.sha256 {
            // Both sides hold the same contents (e.g. an index save lost in a crash).
            let _ = fs::remove_file(&temp.path);
            self.link(rel, item, scanned);
            return Ok(());
        }
        let path = self.local(rel);
        let conflict_rel = match self.conflict_name(rel) {
            Ok(name) => name,
            Err(error) => {
                let _ = fs::remove_file(&temp.path);
                return Err(error);
            }
        };
        let conflict_path = self.local(&conflict_rel);
        if let Err(error) = fs::rename(&path, &conflict_path) {
            let _ = fs::remove_file(&temp.path);
            return Err(format!("{rel}: {error}"));
        }
        if let Err(error) = fs::rename(&temp.path, &path) {
            let _ = fs::rename(&conflict_path, &path);
            let _ = fs::remove_file(&temp.path);
            return Err(format!("{rel}: {error}"));
        }
        self.outcome.conflicts += 1;
        self.record_download(item, temp.file);

        // The conflict copy keeps the local contents and timestamp; upload it as a new file.
        let uploaded = self
            .ensure_remote_dirs(split_rel(&conflict_rel).0)
            .and_then(|()| self.target.upload(&conflict_rel, &conflict_path, None));
        match uploaded {
            Ok(remote) => self.record_upload(&conflict_rel, &remote, scanned),
            // Not indexed: the next run sees it as a new local file and uploads it.
            Err(error) => self.outcome.errors.push(format!("{conflict_rel}: {error}")),
        }
        Ok(())
    }

    fn verify_same(&mut self, rel: &str, item: &RemoteItem) -> Result<(), String> {
        if !self.unchanged_since_scan(rel) {
            return Err(format!(
                "{rel} changed locally during the sync; it will be checked again next run."
            ));
        }
        // Backups never write into the source folder; compare in an app-owned folder.
        let verify_dir = self.recycle_root.join(VERIFY_DIR);
        let temp = match self.job.kind {
            JobKind::Sync => self.download_temp(item, None)?,
            JobKind::Backup => self.download_temp(item, Some(&verify_dir))?,
        };
        let scanned = self.scan.files[rel].clone();
        if temp.file.sha256 == scanned.sha256 {
            let _ = fs::remove_file(&temp.path);
            self.link(rel, item, scanned);
            return Ok(());
        }
        match self.job.kind {
            JobKind::Sync => self.conflict(rel, item, Some(temp)),
            JobKind::Backup => {
                let _ = fs::remove_file(&temp.path);
                self.upload(rel, Some(item.clone()))
            }
        }
    }

    fn trash_remote(&mut self, item: &RemoteItem) -> Result<(), String> {
        let rel = item.rel_path.as_str();
        if exists_no_follow(&self.local(rel)) {
            // Recreated locally since the scan; the next run copies it instead.
            return Ok(());
        }
        if item.is_folder {
            let prefix = format!("{rel}/");
            let not_empty = self
                .remote_all
                .range(prefix.clone()..)
                .next()
                .is_some_and(|path| path.starts_with(&prefix));
            if not_empty {
                return Ok(());
            }
            self.target.trash(item)?;
            self.remote_folders.remove(rel);
            self.remote_all.remove(rel);
            self.index.folders.remove(rel);
        } else {
            self.target.trash(item)?;
            self.remote_all.remove(rel);
            self.index.files.remove(rel);
            self.outcome.trashed += 1;
        }
        Ok(())
    }

    fn conflict_name(&self, rel: &str) -> Result<String, String> {
        let (dir, name) = split_rel(rel);
        let date = date_string(self.now_ms);
        let (stem, extension) = match name.rfind('.') {
            Some(dot) if dot > 0 => name.split_at(dot),
            _ => (name, ""),
        };
        for number in 1..1000 {
            let suffix = if number == 1 {
                format!(" (conflict {date})")
            } else {
                format!(" (conflict {date} {number})")
            };
            let candidate = format!("{stem}{suffix}{extension}");
            if is_ignored_name(&candidate) {
                break;
            }
            let candidate_rel = join_rel(dir, &candidate);
            if !exists_no_follow(&self.local(&candidate_rel))
                && !self.remote_all.contains(&candidate_rel)
            {
                return Ok(candidate_rel);
            }
        }
        Err(format!("{rel}: unable to name a conflict copy."))
    }

    /// Moves a local file into `<recycle root>/<job id>/<date>/<rel>`; never deletes it.
    fn recycle(&self, rel: &str) -> Result<(), String> {
        let source = self.local(rel);
        let base = self
            .recycle_root
            .join(&self.job.id)
            .join(date_string(self.now_ms));
        let destination = unique_path(&local_path(&base, rel));
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{rel}: {e}"))?;
        }
        if fs::rename(&source, &destination).is_ok() {
            return Ok(());
        }
        // Different volume (or rename refused): copy, then remove the original.
        let copied = fs::copy(&source, &destination).map_err(|e| format!("{rel}: {e}"));
        let expected = fs::metadata(&source).map(|m| m.len()).ok();
        match copied {
            Ok(length) if Some(length) == expected => {}
            Ok(_) => {
                let _ = fs::remove_file(&destination);
                return Err(format!("{rel}: unable to move to the recycle folder"));
            }
            Err(error) => {
                let _ = fs::remove_file(&destination);
                return Err(error);
            }
        }
        if let Err(error) = fs::remove_file(&source) {
            let _ = fs::remove_file(&destination);
            return Err(format!("{rel}: {error}"));
        }
        Ok(())
    }
}

fn unique_path(path: &Path) -> PathBuf {
    if !exists_no_follow(path) {
        return path.to_path_buf();
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name.as_str(), ""),
    };
    for number in 2.. {
        let candidate = path.with_file_name(format!("{stem} ({number}){extension}"));
        if !exists_no_follow(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Permanently removes recycle folders older than 30 days. Returns how many were removed.
pub fn purge_recycle(recycle_root: &Path, now_ms: u64) -> Result<usize, String> {
    let jobs = match fs::read_dir(recycle_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    let today = (now_ms / DAY_MS) as i64;
    let mut removed = 0;
    for job in jobs.flatten() {
        let is_job_dir = job.file_type().is_ok_and(|t| t.is_dir())
            && job.file_name().to_str().is_some_and(is_job_id);
        if !is_job_dir {
            continue;
        }
        let Ok(dates) = fs::read_dir(job.path()) else {
            continue;
        };
        for date in dates.flatten() {
            let Some(day) = date.file_name().to_str().and_then(parse_date) else {
                continue;
            };
            if !date.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if today - day > RECYCLE_RETENTION_DAYS {
                fs::remove_dir_all(date.path()).map_err(|e| e.to_string())?;
                removed += 1;
            }
        }
        // Drop the job folder once it is empty.
        let _ = fs::remove_dir(job.path());
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------------------------
// Helpers for the commands
// ---------------------------------------------------------------------------------------------

/// A random 16-character lowercase hex job id.
pub fn new_job_id() -> String {
    random_hex(8)
}

fn random_hex(bytes: usize) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // `RandomState` is seeded from the OS random source; mix in time and a counter.
    let seed = RandomState::new();
    let mut hasher = seed.build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut digest = Sha256::new();
    digest.update(hasher.finish().to_le_bytes());
    digest.update(RandomState::new().build_hasher().finish().to_le_bytes());
    digest.update(nanos.to_le_bytes());
    digest.update(std::process::id().to_le_bytes());
    let digest = digest.finalize();
    to_hex(&digest[..bytes.min(digest.len())])
}

/// Checks a folder chosen for a job: an existing absolute folder that is not a drive root,
/// not the whole home folder, and not inside (or containing) the app's data folder.
pub fn validate_local_path(path: &Path, app_data_dir: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Choose a complete folder path.".into());
    }
    let metadata = fs::metadata(path).map_err(|_| "That folder does not exist.".to_string())?;
    if !metadata.is_dir() {
        return Err("Choose a folder, not a file.".into());
    }
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if canonical.parent().is_none() {
        return Err("Choose a folder, not a whole drive.".into());
    }
    let app_data = fs::canonicalize(app_data_dir).unwrap_or_else(|_| app_data_dir.into());
    if canonical.starts_with(&app_data) || app_data.starts_with(&canonical) {
        return Err("This folder contains pCloud Vault's own data and cannot be synced.".into());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .and_then(|home| fs::canonicalize(home).ok());
    if home.is_some_and(|home| home == canonical) {
        return Err("Choose a folder inside your home folder, not the whole home folder.".into());
    }
    Ok(())
}

fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.into()
    } else {
        format!("{dir}/{name}")
    }
}

/// Splits `a/b/c` into (`a/b`, `c`); a top-level name has an empty directory.
fn split_rel(rel: &str) -> (&str, &str) {
    match rel.rfind('/') {
        Some(slash) => (&rel[..slash], &rel[slash + 1..]),
        None => ("", rel),
    }
}

fn local_path(root: &Path, rel: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in rel.split('/') {
        path.push(component);
    }
    path
}

/// UTC calendar date `YYYY-MM-DD` for a Unix time in milliseconds.
fn date_string(ms: u64) -> String {
    let (year, month, day) = civil_from_days((ms / DAY_MS) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_date(name: &str) -> Option<i64> {
    let bytes = name.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i64 = name[..4].parse().ok()?;
    let month: i64 = name[5..7].parse().ok()?;
    let day: i64 = name[8..].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_794_787_200_000; // 2026-11-16T00:00:00Z
    const TODAY: &str = "2026-11-16";

    // -- Fake remote ----------------------------------------------------------------------

    #[derive(Default)]
    struct FakeTarget {
        items: BTreeMap<String, (RemoteItem, Vec<u8>)>,
        next: u64,
        fail_download: bool,
        trashed: Vec<String>,
        downloads: Vec<PathBuf>,
    }

    impl FakeTarget {
        fn bump(&mut self) -> u64 {
            self.next += 1;
            self.next
        }

        fn put_folder(&mut self, rel: &str) {
            if rel.is_empty() || self.items.contains_key(rel) {
                return;
            }
            self.put_folder(split_rel(rel).0);
            let id = self.bump();
            self.items.insert(
                rel.into(),
                (
                    RemoteItem {
                        rel_path: rel.into(),
                        id: format!("d{id}"),
                        is_folder: true,
                        size: 0,
                        version: String::new(),
                    },
                    Vec::new(),
                ),
            );
        }

        fn put_file(&mut self, rel: &str, data: &[u8]) -> RemoteItem {
            self.put_folder(split_rel(rel).0);
            let version = self.bump();
            let id = match self.items.get(rel) {
                Some((item, _)) => item.id.clone(),
                None => format!("f{}", self.bump()),
            };
            let item = RemoteItem {
                rel_path: rel.into(),
                id,
                is_folder: false,
                size: data.len() as u64,
                version: format!("v{version}"),
            };
            self.items.insert(rel.into(), (item.clone(), data.to_vec()));
            item
        }

        fn data(&self, rel: &str) -> Option<&[u8]> {
            self.items
                .get(rel)
                .filter(|(item, _)| !item.is_folder)
                .map(|(_, data)| data.as_slice())
        }
    }

    impl SyncTarget for FakeTarget {
        fn list_tree(&mut self) -> Result<Vec<RemoteItem>, String> {
            Ok(self.items.values().map(|(item, _)| item.clone()).collect())
        }

        fn upload(
            &mut self,
            rel: &str,
            local: &Path,
            existing: Option<&RemoteItem>,
        ) -> Result<RemoteItem, String> {
            let parent = split_rel(rel).0;
            if !parent.is_empty() && !self.items.get(parent).is_some_and(|(i, _)| i.is_folder) {
                return Err("parent missing".into());
            }
            if let Some(existing) = existing {
                assert_eq!(self.items[rel].0.id, existing.id);
            }
            let data = fs::read(local).map_err(|e| e.to_string())?;
            Ok(self.put_file(rel, &data))
        }

        fn download(&mut self, item: &RemoteItem, dest: &Path) -> Result<(), String> {
            let data = self.items[&item.rel_path].1.clone();
            self.downloads.push(dest.to_path_buf());
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dest)
                .map_err(|e| e.to_string())?;
            if self.fail_download {
                file.write_all(&data[..data.len() / 2]).unwrap();
                return Err("connection lost".into());
            }
            file.write_all(&data).map_err(|e| e.to_string())
        }

        fn create_folder(&mut self, rel: &str) -> Result<RemoteItem, String> {
            let parent = split_rel(rel).0;
            if !parent.is_empty() && !self.items.contains_key(parent) {
                return Err("parent missing".into());
            }
            self.put_folder(rel);
            Ok(self.items[rel].0.clone())
        }

        fn trash(&mut self, item: &RemoteItem) -> Result<(), String> {
            let prefix = format!("{}/", item.rel_path);
            self.items
                .retain(|path, _| path != &item.rel_path && !path.starts_with(&prefix));
            self.trashed.push(item.rel_path.clone());
            Ok(())
        }
    }

    // -- Helpers ----------------------------------------------------------------------------

    fn file(sha: &str, size: u64) -> LocalFile {
        LocalFile {
            size,
            mtime_ms: 1,
            sha256: sha.into(),
        }
    }

    fn entry(sha: &str, id: &str, version: &str) -> IndexEntry {
        IndexEntry {
            local_size: 1,
            local_mtime_ms: 1,
            local_sha256: sha.into(),
            remote_id: id.into(),
            remote_version: version.into(),
        }
    }

    fn remote_file(rel: &str, id: &str, version: &str, size: u64) -> RemoteItem {
        RemoteItem {
            rel_path: rel.into(),
            id: id.into(),
            is_folder: false,
            size,
            version: version.into(),
        }
    }

    fn remote_folder(rel: &str) -> RemoteItem {
        RemoteItem {
            rel_path: rel.into(),
            id: format!("d-{rel}"),
            is_folder: true,
            size: 0,
            version: String::new(),
        }
    }

    fn scan_of(files: &[(&str, LocalFile)]) -> LocalScan {
        LocalScan {
            files: files
                .iter()
                .map(|(rel, file)| (rel.to_string(), file.clone()))
                .collect(),
            ..LocalScan::default()
        }
    }

    fn index_of(files: &[(&str, IndexEntry)]) -> JobIndex {
        JobIndex {
            files: files
                .iter()
                .map(|(rel, entry)| (rel.to_string(), entry.clone()))
                .collect(),
            folders: BTreeSet::new(),
        }
    }

    fn sync_plan(local: &LocalScan, index: &JobIndex, remote: &[RemoteItem]) -> Vec<Action> {
        plan(JobKind::Sync, false, local, index, remote).actions
    }

    fn job(kind: JobKind, local_path: &Path) -> FolderJob {
        FolderJob {
            id: "0123456789abcdef".into(),
            kind,
            local_path: local_path.into(),
            location: TargetLocation::Vault,
            folder_id: None,
            paused: false,
            mirror_deletions: false,
            last_run: None,
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        local: PathBuf,
        recycle: PathBuf,
        store: JsonIndexStore,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("local");
        fs::create_dir(&local).unwrap();
        Fixture {
            local,
            recycle: temp.path().join("recycle"),
            store: JsonIndexStore {
                dir: temp.path().join("index"),
            },
            _temp: temp,
        }
    }

    fn execute_job(
        fixture: &Fixture,
        job: &FolderJob,
        target: &mut FakeTarget,
        allow_deletes: bool,
    ) -> RunOutcome {
        run(
            job,
            target,
            &fixture.store,
            &fixture.recycle,
            allow_deletes,
            &mut |_| {},
            NOW,
        )
        .unwrap()
    }

    fn write(path: &Path, data: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    /// Changes a file and moves its timestamp so size+mtime detection sees it.
    fn modify(path: &Path, data: &[u8]) {
        fs::write(path, data).unwrap();
        let file = OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + std::time::Duration::from_secs(120))
            .unwrap();
    }

    // -- Planner ----------------------------------------------------------------------------

    #[test]
    fn folders_sync_new_files_copy_both_ways() {
        let local = scan_of(&[("a.txt", file("h1", 3))]);
        let remote = [remote_file("b.txt", "f1", "v1", 3)];
        let actions = sync_plan(&local, &JobIndex::default(), &remote);
        assert_eq!(
            actions,
            vec![
                Action::Upload {
                    rel: "a.txt".into(),
                    existing: None
                },
                Action::Download {
                    item: remote[0].clone()
                },
            ]
        );
    }

    #[test]
    fn folders_sync_changes_flow_from_the_changed_side() {
        let index = index_of(&[
            ("local.txt", entry("old", "f1", "v1")),
            ("remote.txt", entry("same", "f2", "v1")),
            ("both.txt", entry("old", "f3", "v1")),
            ("none.txt", entry("same", "f4", "v1")),
        ]);
        let local = scan_of(&[
            ("local.txt", file("new", 1)),
            ("remote.txt", file("same", 1)),
            ("both.txt", file("new", 1)),
            ("none.txt", file("same", 1)),
        ]);
        let remote = [
            remote_file("local.txt", "f1", "v1", 1),
            remote_file("remote.txt", "f2", "v2", 1),
            remote_file("both.txt", "f3", "v2", 1),
            remote_file("none.txt", "f4", "v1", 1),
        ];
        let actions = sync_plan(&local, &index, &remote);
        assert_eq!(
            actions,
            vec![
                Action::ConflictKeepBoth {
                    rel: "both.txt".into(),
                    item: remote[2].clone()
                },
                Action::Upload {
                    rel: "local.txt".into(),
                    existing: Some(remote[0].clone())
                },
                Action::Download {
                    item: remote[1].clone()
                },
            ]
        );
    }

    #[test]
    fn folders_first_run_same_path_verifies_equal_sizes_and_conflicts_otherwise() {
        let local = scan_of(&[("same.txt", file("h", 5)), ("diff.txt", file("h", 5))]);
        let remote = [
            remote_file("same.txt", "f1", "v1", 5),
            remote_file("diff.txt", "f2", "v1", 9),
        ];
        let actions = sync_plan(&local, &JobIndex::default(), &remote);
        assert_eq!(
            actions,
            vec![
                Action::ConflictKeepBoth {
                    rel: "diff.txt".into(),
                    item: remote[1].clone()
                },
                Action::VerifySame {
                    rel: "same.txt".into(),
                    item: remote[0].clone()
                },
            ]
        );
        let backup = plan(
            JobKind::Backup,
            false,
            &local,
            &JobIndex::default(),
            &remote,
        );
        assert_eq!(
            backup.actions[0],
            Action::Upload {
                rel: "diff.txt".into(),
                existing: Some(remote[1].clone())
            }
        );
    }

    #[test]
    fn folders_sync_deletions_propagate_unless_the_other_side_changed() {
        let index = index_of(&[
            ("gone-local.txt", entry("h", "f1", "v1")),
            ("gone-local-edited-remote.txt", entry("h", "f2", "v1")),
            ("gone-remote.txt", entry("h", "f3", "v1")),
            ("gone-remote-edited-local.txt", entry("h", "f4", "v1")),
            ("gone-both.txt", entry("h", "f5", "v1")),
        ]);
        let local = scan_of(&[
            ("gone-remote.txt", file("h", 1)),
            ("gone-remote-edited-local.txt", file("h2", 1)),
        ]);
        let remote = [
            remote_file("gone-local.txt", "f1", "v1", 1),
            remote_file("gone-local-edited-remote.txt", "f2", "v2", 1),
        ];
        let actions = sync_plan(&local, &index, &remote);
        assert_eq!(
            actions,
            vec![
                Action::Download {
                    item: remote[1].clone()
                },
                Action::Upload {
                    rel: "gone-remote-edited-local.txt".into(),
                    existing: None
                },
                Action::TrashRemote {
                    item: remote[0].clone()
                },
                Action::RecycleLocal {
                    rel: "gone-remote.txt".into()
                },
                Action::ForgetIndex {
                    rel: "gone-both.txt".into()
                },
            ]
        );
    }

    #[test]
    fn folders_backup_never_touches_local_files() {
        let index = index_of(&[
            ("remote-edited.txt", entry("h", "f1", "v1")),
            ("remote-deleted.txt", entry("h", "f2", "v1")),
            ("local-deleted.txt", entry("h", "f3", "v1")),
        ]);
        let local = scan_of(&[
            ("remote-edited.txt", file("h", 1)),
            ("remote-deleted.txt", file("h", 1)),
        ]);
        let remote = [
            remote_file("remote-edited.txt", "f1", "v2", 1),
            remote_file("local-deleted.txt", "f3", "v1", 1),
            remote_file("remote-new.txt", "f4", "v1", 1),
        ];
        let actions = plan(JobKind::Backup, false, &local, &index, &remote).actions;
        assert_eq!(
            actions,
            vec![
                Action::Upload {
                    rel: "remote-deleted.txt".into(),
                    existing: None
                },
                Action::ForgetIndex {
                    rel: "local-deleted.txt".into()
                },
            ]
        );
        assert!(!actions.iter().any(|a| matches!(
            a,
            Action::Download { .. }
                | Action::RecycleLocal { .. }
                | Action::RemoveLocalFolder { .. }
                | Action::CreateLocalFolder { .. }
                | Action::ConflictKeepBoth { .. }
        )));
    }

    #[test]
    fn folders_backup_mirrors_deletions_only_when_enabled() {
        let index = index_of(&[
            ("deleted.txt", entry("h", "f1", "v1")),
            ("deleted-edited-remote.txt", entry("h", "f2", "v1")),
        ]);
        let remote = [
            remote_file("deleted.txt", "f1", "v1", 1),
            remote_file("deleted-edited-remote.txt", "f2", "v2", 1),
        ];
        let actions = plan(
            JobKind::Backup,
            true,
            &LocalScan::default(),
            &index,
            &remote,
        )
        .actions;
        assert_eq!(
            actions,
            vec![
                Action::TrashRemote {
                    item: remote[0].clone()
                },
                Action::ForgetIndex {
                    rel: "deleted-edited-remote.txt".into()
                },
            ]
        );
    }

    #[test]
    fn folders_folder_rules() {
        let mut local = LocalScan::default();
        local
            .folders
            .extend(["l-new".into(), "both".into(), "r-gone".into()]);
        let mut index = JobIndex::default();
        index.folders.extend([
            "both".into(),
            "r-gone".into(),
            "l-gone".into(),
            "all-gone".into(),
        ]);
        let remote = [
            remote_folder("both"),
            remote_folder("r-new"),
            remote_folder("l-gone"),
        ];
        let actions = sync_plan(&local, &index, &remote);
        assert_eq!(
            actions,
            vec![
                Action::CreateRemoteFolder {
                    rel: "l-new".into()
                },
                Action::CreateLocalFolder {
                    rel: "r-new".into()
                },
                Action::ForgetIndex {
                    rel: "all-gone".into()
                },
                Action::RemoveLocalFolder {
                    rel: "r-gone".into()
                },
                Action::TrashRemote {
                    item: remote[2].clone()
                },
            ]
        );
        let backup = plan(JobKind::Backup, false, &local, &index, &remote).actions;
        assert_eq!(
            backup,
            vec![
                Action::CreateRemoteFolder {
                    rel: "l-new".into()
                },
                Action::CreateRemoteFolder {
                    rel: "r-gone".into()
                },
                Action::ForgetIndex {
                    rel: "all-gone".into()
                },
                Action::ForgetIndex {
                    rel: "l-gone".into()
                },
            ]
        );
    }

    #[test]
    fn folders_ignored_and_skipped_paths_are_left_alone() {
        for name in [
            "~$report.docx",
            "a.TMP",
            "video.part",
            ".~lock.file#",
            ".DS_Store",
            "desktop.ini",
            "Thumbs.db",
            ".pcloud-vault-partial-00",
            "bad:name",
            "CON",
        ] {
            assert!(is_ignored_name(name), "{name}");
        }
        assert!(!is_ignored_name("report.docx"));

        let mut local = LocalScan::default();
        local.skipped.insert("linked".into());
        let index = index_of(&[("linked/a.txt", entry("h", "f1", "v1"))]);
        let remote = [
            remote_file("x.tmp", "f2", "v1", 1),
            remote_file("dir/~$lock.docx", "f3", "v1", 1),
            remote_file("linked/a.txt", "f1", "v1", 1),
            remote_folder("linked"),
        ];
        assert!(sync_plan(&local, &index, &remote).is_empty());
    }

    #[test]
    fn folders_delete_guard_threshold() {
        assert!(!exceeds_delete_guard(5, 6));
        assert!(exceeds_delete_guard(6, 10));
        assert!(!exceeds_delete_guard(6, 24));
        assert!(exceeds_delete_guard(6, 23));
        assert!(!exceeds_delete_guard(50, 1000));
        assert!(exceeds_delete_guard(51, 1000));
    }

    #[test]
    fn folders_restore_turns_deletions_into_copies() {
        let item = remote_file("a.txt", "f1", "v1", 1);
        let folder = remote_folder("dir");
        let plan = Plan {
            actions: vec![
                Action::TrashRemote { item: item.clone() },
                Action::RecycleLocal {
                    rel: "b.txt".into(),
                },
                Action::RemoveLocalFolder { rel: "gone".into() },
                Action::TrashRemote {
                    item: folder.clone(),
                },
            ],
            notes: Vec::new(),
        };
        assert_eq!(
            plan_restore(&plan).actions,
            vec![
                Action::CreateLocalFolder { rel: "dir".into() },
                Action::CreateRemoteFolder { rel: "gone".into() },
                Action::Download { item },
                Action::Upload {
                    rel: "b.txt".into(),
                    existing: None
                },
            ]
        );
    }

    // -- Scanner and helpers ----------------------------------------------------------------

    #[test]
    fn folders_scan_skips_temp_files_and_reuses_hashes() {
        let fixture = fixture();
        write(&fixture.local.join("keep.txt"), b"keep");
        write(&fixture.local.join("sub/inner.txt"), b"inner");
        for name in [
            "~$x.docx",
            "y.tmp",
            "z.part",
            "desktop.ini",
            ".pcloud-vault-partial-1",
        ] {
            write(&fixture.local.join(name), b"temp");
        }
        let scan = scan_local(&fixture.local, &JobIndex::default()).unwrap();
        assert_eq!(
            scan.files.keys().collect::<Vec<_>>(),
            vec!["keep.txt", "sub/inner.txt"]
        );
        assert_eq!(scan.folders, BTreeSet::from(["sub".to_string()]));
        assert_eq!(
            scan.files["keep.txt"].sha256,
            to_hex(&Sha256::digest(b"keep"))
        );

        let mut index = JobIndex::default();
        let known = &scan.files["keep.txt"];
        index.files.insert(
            "keep.txt".into(),
            IndexEntry {
                local_size: known.size,
                local_mtime_ms: known.mtime_ms,
                local_sha256: "cached".into(),
                ..IndexEntry::default()
            },
        );
        let rescan = scan_local(&fixture.local, &index).unwrap();
        assert_eq!(rescan.files["keep.txt"].sha256, "cached");
    }

    #[test]
    fn folders_dates_and_ids() {
        assert_eq!(date_string(0), "1970-01-01");
        assert_eq!(date_string(NOW), TODAY);
        assert_eq!(date_string(951_782_400_000), "2000-02-29");
        assert_eq!(
            parse_date("2000-02-29"),
            Some(951_782_400_000 / DAY_MS as i64)
        );
        assert_eq!(parse_date("nope"), None);
        let a = new_job_id();
        let b = new_job_id();
        assert!(is_job_id(&a) && is_job_id(&b));
        assert_ne!(a, b);
        assert_eq!(cloud_folder_id(None), Ok(0));
        assert_eq!(cloud_folder_id(Some("d42")), Ok(42));
        assert!(cloud_folder_id(Some("f42")).is_err());
    }

    #[test]
    fn folders_validate_local_path_rejects_unsafe_choices() {
        let fixture = fixture();
        let app_data = fixture._temp.path().join("appdata");
        fs::create_dir(&app_data).unwrap();
        assert!(validate_local_path(&fixture.local, &app_data).is_ok());
        assert!(validate_local_path(Path::new("relative"), &app_data).is_err());
        assert!(validate_local_path(&fixture.local.join("missing"), &app_data).is_err());
        assert!(validate_local_path(&app_data, &app_data).is_err());
        assert!(validate_local_path(fixture._temp.path(), &app_data).is_err());
        let root = fixture.local.ancestors().last().unwrap().to_path_buf();
        assert!(validate_local_path(&root, &app_data).is_err());
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            assert!(validate_local_path(Path::new(&home), &app_data).is_err());
        }
    }

    #[test]
    fn folders_purge_recycle_removes_only_old_dated_folders() {
        let fixture = fixture();
        let job_dir = fixture.recycle.join("0123456789abcdef");
        write(&job_dir.join("2026-10-01/old.txt"), b"x");
        write(&job_dir.join("2026-11-01/recent.txt"), b"x");
        write(&job_dir.join("not-a-date/keep.txt"), b"x");
        write(&fixture.recycle.join("other/2020-01-01/keep.txt"), b"x");
        assert_eq!(purge_recycle(&fixture.recycle, NOW).unwrap(), 1);
        assert!(!job_dir.join("2026-10-01").exists());
        assert!(job_dir.join("2026-11-01/recent.txt").exists());
        assert!(job_dir.join("not-a-date/keep.txt").exists());
        assert!(fixture.recycle.join("other/2020-01-01/keep.txt").exists());
        assert_eq!(purge_recycle(&fixture.local.join("missing"), NOW), Ok(0));
    }

    // -- Executor ---------------------------------------------------------------------------

    #[test]
    fn folders_missing_local_folder_is_an_error_not_deletions() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("a.txt"), b"a");
        let job = job(JobKind::Sync, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);
        assert!(target.data("a.txt").is_some());
        let index_before = fixture.store.load(&job.id).unwrap();

        fs::remove_dir_all(&fixture.local).unwrap();
        let error = run(
            &job,
            &mut target,
            &fixture.store,
            &fixture.recycle,
            true,
            &mut |_| {},
            NOW,
        )
        .unwrap_err();
        assert_eq!(error, LOCAL_FOLDER_UNAVAILABLE);
        assert!(target.trashed.is_empty());
        assert_eq!(target.data("a.txt"), Some(&b"a"[..]));
        assert_eq!(fixture.store.load(&job.id).unwrap(), index_before);
    }

    #[test]
    fn folders_sync_roundtrip_both_ways() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("docs/local.txt"), b"from local");
        target.put_file("photos/remote.jpg", b"from remote");
        let job = job(JobKind::Sync, &fixture.local);

        let mut events = Vec::new();
        let outcome = run(
            &job,
            &mut target,
            &fixture.store,
            &fixture.recycle,
            false,
            &mut |p| events.push(p),
            NOW,
        )
        .unwrap();
        assert_eq!((outcome.uploaded, outcome.downloaded), (1, 1));
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(outcome.file_count, 2);
        assert_eq!(events.len(), 2);
        assert_eq!(target.data("docs/local.txt"), Some(&b"from local"[..]));
        assert_eq!(
            fs::read(fixture.local.join("photos/remote.jpg")).unwrap(),
            b"from remote"
        );

        // Nothing changed: nothing transferred.
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!((outcome.uploaded, outcome.downloaded), (0, 0));

        // Local edit uploads; remote edit downloads and recycles the replaced local copy.
        modify(&fixture.local.join("docs/local.txt"), b"edited locally");
        target.put_file("photos/remote.jpg", b"edited remotely");
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!((outcome.uploaded, outcome.downloaded), (1, 1));
        assert_eq!(target.data("docs/local.txt"), Some(&b"edited locally"[..]));
        assert_eq!(
            fs::read(fixture.local.join("photos/remote.jpg")).unwrap(),
            b"edited remotely"
        );
        let recycled = fixture
            .recycle
            .join(&job.id)
            .join(TODAY)
            .join("photos/remote.jpg");
        assert_eq!(fs::read(recycled).unwrap(), b"from remote");
    }

    #[test]
    fn folders_conflict_keeps_both_versions() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("notes.txt"), b"base");
        let job = job(JobKind::Sync, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);

        modify(&fixture.local.join("notes.txt"), b"local edit");
        target.put_file("notes.txt", b"remote edit");
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(outcome.conflicts, 1);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let conflict = format!("notes (conflict {TODAY}).txt");
        assert_eq!(
            fs::read(fixture.local.join("notes.txt")).unwrap(),
            b"remote edit"
        );
        assert_eq!(
            fs::read(fixture.local.join(&conflict)).unwrap(),
            b"local edit"
        );
        assert_eq!(target.data(&conflict), Some(&b"local edit"[..]));
        assert_eq!(target.data("notes.txt"), Some(&b"remote edit"[..]));

        // Both sides now agree; the next run is quiet.
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(
            (outcome.uploaded, outcome.downloaded, outcome.conflicts),
            (0, 0, 0)
        );
    }

    #[test]
    fn folders_first_run_links_identical_files_and_splits_different_ones() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("same.txt"), b"12345");
        write(&fixture.local.join("differs.txt"), b"aaaaa");
        target.put_file("same.txt", b"12345");
        target.put_file("differs.txt", b"bbbbb");
        let job = job(JobKind::Sync, &fixture.local);
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(outcome.conflicts, 1);
        assert_eq!(outcome.uploaded, 1);
        let index = fixture.store.load(&job.id).unwrap();
        assert!(index.files.contains_key("same.txt"));
        assert_eq!(
            fs::read(fixture.local.join("differs.txt")).unwrap(),
            b"bbbbb"
        );
        let conflict = format!("differs (conflict {TODAY}).txt");
        assert_eq!(target.data(&conflict), Some(&b"aaaaa"[..]));
    }

    #[test]
    fn folders_remote_deletion_recycles_local_file() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("dir/old.txt"), b"old");
        let job = job(JobKind::Sync, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);

        let item = target.items["dir"].0.clone();
        target.trash(&item).unwrap();
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(outcome.recycled, 1);
        assert!(!fixture.local.join("dir").exists());
        let recycled = fixture
            .recycle
            .join(&job.id)
            .join(TODAY)
            .join("dir/old.txt");
        assert_eq!(fs::read(recycled).unwrap(), b"old");
        let index = fixture.store.load(&job.id).unwrap();
        assert!(index.files.is_empty() && index.folders.is_empty());
    }

    #[test]
    fn folders_failed_download_leaves_no_partial_file() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        target.put_file("big.bin", &[7u8; 1000]);
        target.fail_download = true;
        let job = job(JobKind::Sync, &fixture.local);
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(outcome.downloaded, 0);
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(fs::read_dir(&fixture.local).unwrap().count(), 0);

        // Same when replacing an existing file: the old copy stays in place.
        target.fail_download = false;
        execute_job(&fixture, &job, &mut target, false);
        target.put_file("big.bin", &[9u8; 1000]);
        target.fail_download = true;
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(
            fs::read(fixture.local.join("big.bin")).unwrap(),
            vec![7u8; 1000]
        );
        assert_eq!(fs::read_dir(&fixture.local).unwrap().count(), 1);
    }

    #[test]
    fn folders_index_is_persisted_between_runs() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("a.txt"), b"alpha");
        let job = job(JobKind::Backup, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);
        let index = fixture.store.load(&job.id).unwrap();
        let saved = &index.files["a.txt"];
        assert_eq!(saved.local_sha256, to_hex(&Sha256::digest(b"alpha")));
        assert_eq!(saved.remote_id, target.items["a.txt"].0.id);
        assert_eq!(saved.remote_version, target.items["a.txt"].0.version);
        assert!(fixture.dir_has_index(&job.id));

        fixture.store.remove(&job.id).unwrap();
        assert_eq!(fixture.store.load(&job.id).unwrap(), JobIndex::default());
    }

    #[test]
    fn folders_mass_deletion_is_held_until_confirmed_or_restored() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        for n in 0..10 {
            write(&fixture.local.join(format!("f{n}.txt")), b"data");
        }
        write(&fixture.local.join("new.txt"), b"x");
        let job = job(JobKind::Sync, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);
        for n in 0..10 {
            fs::remove_file(fixture.local.join(format!("f{n}.txt"))).unwrap();
        }
        write(&fixture.local.join("later.txt"), b"y");

        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(outcome.pending_deletes, 10);
        assert_eq!(outcome.uploaded, 1, "other actions still run");
        assert!(target.trashed.is_empty());
        assert_eq!(
            JobStatus::from_outcome(&outcome).state,
            JobState::NeedsReview
        );

        let restored = run_restore(
            &job,
            &mut target,
            &fixture.store,
            &fixture.recycle,
            &mut |_| {},
            NOW,
        )
        .unwrap();
        assert_eq!(restored.downloaded, 10);
        assert!(fixture.local.join("f3.txt").exists());

        for n in 0..10 {
            fs::remove_file(fixture.local.join(format!("f{n}.txt"))).unwrap();
        }
        let outcome = execute_job(&fixture, &job, &mut target, true);
        assert_eq!((outcome.trashed, outcome.pending_deletes), (10, 0));
        assert_eq!(target.trashed.len(), 10);
    }

    #[test]
    fn folders_case_only_differences_are_left_alone() {
        let local = scan_of(&[("A.txt", file("h2", 1)), ("b.txt", file("h", 1))]);
        let index = index_of(&[("a.txt", entry("h", "f1", "v1"))]);
        let remote = [
            remote_file("a.txt", "f1", "v1", 1),
            remote_file("B.txt", "f2", "v1", 1),
        ];
        let plan = plan(JobKind::Sync, false, &local, &index, &remote);
        assert!(plan.actions.is_empty(), "{:?}", plan.actions);
        assert_eq!(plan.notes.len(), 4);
    }

    #[test]
    fn folders_emptied_local_folder_holds_even_small_deletions() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("a.txt"), b"a");
        write(&fixture.local.join("b.txt"), b"b");
        let job = job(JobKind::Sync, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);
        fs::remove_file(fixture.local.join("a.txt")).unwrap();
        fs::remove_file(fixture.local.join("b.txt")).unwrap();

        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!((outcome.pending_deletes, outcome.trashed), (2, 0));
        assert!(target.data("a.txt").is_some());
        let outcome = execute_job(&fixture, &job, &mut target, true);
        assert_eq!(outcome.trashed, 2);
    }

    #[test]
    fn folders_identical_edits_on_both_sides_do_not_create_conflict_copies() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("same.txt"), b"base");
        let job = job(JobKind::Sync, &fixture.local);
        execute_job(&fixture, &job, &mut target, false);
        modify(&fixture.local.join("same.txt"), b"identical edit");
        target.put_file("same.txt", b"identical edit");

        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(
            (outcome.conflicts, outcome.uploaded, outcome.downloaded),
            (0, 0, 0)
        );
        assert_eq!(fs::read_dir(&fixture.local).unwrap().count(), 1);
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!((outcome.uploaded, outcome.downloaded), (0, 0));
    }

    #[test]
    fn folders_backup_verification_does_not_write_into_the_source() {
        let fixture = fixture();
        let mut target = FakeTarget::default();
        write(&fixture.local.join("same.txt"), b"12345");
        write(&fixture.local.join("differs.txt"), b"aaaaa");
        target.put_file("same.txt", b"12345");
        target.put_file("differs.txt", b"bbbbb");
        let job = job(JobKind::Backup, &fixture.local);
        let outcome = execute_job(&fixture, &job, &mut target, false);
        assert_eq!(target.downloads.len(), 2);
        assert!(target
            .downloads
            .iter()
            .all(|path| path.starts_with(fixture.recycle.join(VERIFY_DIR))));
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!((outcome.uploaded, outcome.downloaded), (1, 0));
        assert_eq!(target.data("differs.txt"), Some(&b"aaaaa"[..]));
        assert_eq!(
            fs::read(fixture.local.join("differs.txt")).unwrap(),
            b"aaaaa"
        );
        assert_eq!(fs::read_dir(&fixture.local).unwrap().count(), 2);
        assert_eq!(
            fs::read_dir(fixture.recycle.join(VERIFY_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    impl Fixture {
        fn dir_has_index(&self, job_id: &str) -> bool {
            self.store.dir.join(format!("{job_id}.json")).is_file()
        }
    }
}
