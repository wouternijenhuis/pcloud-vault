//! Backup and sync jobs: commands, status and the background scheduler around the
//! engine in `folders.rs`.

use crate::app::{cloud_safe, locked, now, path_key, task, AppResult, AppState, Inner};
use crate::features::destination_allowed;
use crate::folders::{
    self, CloudTarget, FolderJob, JobKind, JobState, JobStatus, JsonIndexStore, RunOutcome,
    TargetLocation, VaultIndexStore, VaultTarget,
};
use crate::transfers::Direction;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Manager, State};

/// Seconds between automatic runs while the window is open, and while it is hidden.
const VISIBLE_INTERVAL_MS: u64 = 30_000;
const BACKGROUND_INTERVAL_MS: u64 = 120_000;

#[derive(Default)]
pub struct JobRuntime {
    status: Mutex<HashMap<String, JobStatus>>,
    running: Mutex<HashSet<String>>,
    last_attempt: Mutex<HashMap<String, u64>>,
    labels: Mutex<HashMap<String, String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetView {
    location: &'static str,
    folder_id: Option<String>,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    id: String,
    kind: JobKind,
    local_path: String,
    target: TargetView,
    paused: bool,
    mirror_deletions: bool,
    last_run: Option<u64>,
    state: JobState,
    message: Option<String>,
    pending_deletes: usize,
    file_count: usize,
}

fn index_dir(s: &Inner) -> PathBuf {
    s.directory.join("jobs")
}
fn recycle_root(s: &Inner) -> PathBuf {
    s.directory.join("recycle")
}

fn job(s: &Inner, id: &str) -> AppResult<FolderJob> {
    locked(&s.settings)?
        .jobs
        .iter()
        .find(|j| j.id == id)
        .cloned()
        .ok_or_else(|| "This backup or sync no longer exists.".into())
}

fn label(s: &Inner, job: &FolderJob) -> String {
    if let Some(label) = locked(&s.jobs.labels)
        .ok()
        .and_then(|l| l.get(&job.id).cloned())
    {
        return label;
    }
    let label = match job.location {
        TargetLocation::Vault => {
            let mut parts = Vec::new();
            if let Ok(vault) = locked(&s.vault) {
                if let Some(vault) = vault.as_ref() {
                    let mut current = job.folder_id.clone();
                    while let Some(id) = current {
                        match vault.entry(&id) {
                            Ok(entry) => {
                                parts.push(entry.name);
                                current = entry.parent_id;
                            }
                            Err(_) => break,
                        }
                    }
                } else {
                    return "Vault".into();
                }
            }
            parts.push("Vault".into());
            parts.reverse();
            parts.join(" / ")
        }
        TargetLocation::Cloud => {
            let name = folders::cloud_folder_id(job.folder_id.as_deref())
                .ok()
                .and_then(|id| s.client().ok().and_then(|c| c.stat(id, true).ok()))
                .map(|m| m.name);
            match name {
                Some(name) => format!("pCloud / {name}"),
                None => return "pCloud".into(),
            }
        }
    };
    if let Ok(mut labels) = locked(&s.jobs.labels) {
        labels.insert(job.id.clone(), label.clone());
    }
    label
}

fn view(s: &Inner, job: &FolderJob) -> JobView {
    let status = locked(&s.jobs.status)
        .ok()
        .and_then(|m| m.get(&job.id).cloned())
        .unwrap_or_else(|| JobStatus::new(JobState::Idle));
    let state = if job.paused {
        JobState::Paused
    } else {
        status.state
    };
    JobView {
        id: job.id.clone(),
        kind: job.kind,
        local_path: job.local_path.to_string_lossy().into_owned(),
        target: TargetView {
            location: match job.location {
                TargetLocation::Vault => "vault",
                TargetLocation::Cloud => "cloud",
            },
            folder_id: job.folder_id.clone(),
            label: label(s, job),
        },
        paused: job.paused,
        mirror_deletions: job.mirror_deletions,
        last_run: job.last_run,
        state,
        message: status.message,
        pending_deletes: status.pending_deletes,
        file_count: status.file_count,
    }
}

fn set_status(s: &Inner, id: &str, status: JobStatus) {
    if let Ok(mut map) = locked(&s.jobs.status) {
        map.insert(id.into(), status);
    }
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>) -> AppResult<Vec<JobView>> {
    task(&state, move |s| {
        let jobs = locked(&s.settings)?.jobs.clone();
        Ok(jobs.iter().map(|j| view(&s, j)).collect())
    })
    .await
}

/// Resolves the destination folder, creating one named after the local folder when the
/// user picked the Vault or pCloud root, so a job never mixes with unrelated files.
fn resolve_destination(
    s: &Arc<Inner>,
    location: TargetLocation,
    folder_id: Option<String>,
    local: &Path,
) -> AppResult<Option<String>> {
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Choose a named folder on this computer.")?
        .to_string();
    match location {
        TargetLocation::Vault => {
            let mut vault = locked(&s.vault)?;
            let vault = vault.as_mut().ok_or("Unlock your Vault first.")?;
            if let Some(id) = &folder_id {
                let entry = vault.entry(id).map_err(|e| e.to_string())?;
                if entry.kind != vault_core::EntryKind::Folder || entry.trashed {
                    return Err("Choose a Vault folder.".into());
                }
                return Ok(folder_id);
            }
            let existing = vault
                .list(None, false)
                .into_iter()
                .find(|e| e.kind == vault_core::EntryKind::Folder && e.name == name);
            let folder = match existing {
                Some(folder) => folder,
                None => vault
                    .create_folder(None, &name)
                    .map_err(|e| e.to_string())?,
            };
            Ok(Some(folder.id))
        }
        TargetLocation::Cloud => {
            let client = s.client()?;
            let reserved = locked(&s.settings)?.remote_folder;
            if let Some(id) = &folder_id {
                let number = folders::cloud_folder_id(Some(id))?;
                if number == 0 {
                    return resolve_destination(s, location, None, local);
                }
                cloud_safe(s, &client, number, true)?;
                if reserved.is_some_and(|r| {
                    client
                        .stat(r, true)
                        .is_ok_and(|m| m.parent_folder_id == number)
                }) {
                    return Err(
                        "Choose a folder that doesn't contain the Vault's own storage.".into(),
                    );
                }
                return Ok(folder_id);
            }
            let existing = client
                .list_folder(0)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|m| m.is_folder && m.name == name && Some(m.id) != reserved);
            let folder = match existing {
                Some(folder) => folder,
                None => client.create_folder(0, &name).map_err(|e| e.to_string())?,
            };
            Ok(Some(format!("d{}", folder.id)))
        }
    }
}

#[tauri::command]
pub async fn add_job(
    state: State<'_, AppState>,
    kind: JobKind,
    local_path: String,
    location: TargetLocation,
    folder_id: Option<String>,
    mirror_deletions: bool,
    replaces: Option<String>,
) -> AppResult<JobView> {
    let app = state.0.clone();
    let view = task(&state, move |s| {
        destination_allowed(&s, location == TargetLocation::Vault)?;
        let local = PathBuf::from(&local_path);
        folders::validate_local_path(&local, &s.directory)?;
        let local = std::fs::canonicalize(&local).map_err(|e| e.to_string())?;
        if let Some(old) = &replaces {
            if locked(&s.jobs.running)?.contains(old) {
                return Err("Wait until the current backup or sync finishes.".into());
            }
        }
        for other in locked(&s.settings)?
            .jobs
            .iter()
            .filter(|j| Some(&j.id) != replaces.as_ref())
        {
            let other = std::fs::canonicalize(&other.local_path).unwrap_or(other.local_path.clone());
            if path_key(&local).starts_with(path_key(&other))
                || path_key(&other).starts_with(path_key(&local))
            {
                return Err("This folder overlaps a folder that is already backed up or synced. Choose a different folder.".into());
            }
        }
        let folder_id = resolve_destination(&s, location, folder_id, &local)?;
        let job = FolderJob {
            id: folders::new_job_id(),
            kind,
            local_path: local,
            location,
            folder_id,
            paused: false,
            mirror_deletions: kind == JobKind::Backup && mirror_deletions,
            last_run: None,
        };
        {
            let mut settings = locked(&s.settings)?;
            if let Some(old) = &replaces {
                settings.jobs.retain(|j| &j.id != old);
            }
            settings.jobs.push(job.clone());
        }
        s.save_settings()?;
        if let Some(old) = &replaces {
            forget_index(&s, old);
        }
        s.log(
            match (kind, location) {
                (JobKind::Sync, TargetLocation::Vault) => "Added an encrypted sync.",
                (JobKind::Sync, TargetLocation::Cloud) => "Added a sync to pCloud without encryption.",
                (JobKind::Backup, TargetLocation::Vault) => "Added an encrypted backup.",
                (JobKind::Backup, TargetLocation::Cloud) => "Added a backup to pCloud without encryption.",
            },
            "success",
        );
        Ok((view(&s, &job), job.id))
    })
    .await?;
    spawn_run(app, view.1, RunMode::Normal);
    Ok(view.0)
}

#[tauri::command]
pub async fn update_job(
    state: State<'_, AppState>,
    id: String,
    paused: Option<bool>,
    mirror_deletions: Option<bool>,
) -> AppResult<JobView> {
    task(&state, move |s| {
        let job = {
            let mut settings = locked(&s.settings)?;
            let job = settings
                .jobs
                .iter_mut()
                .find(|j| j.id == id)
                .ok_or("This backup or sync no longer exists.")?;
            if let Some(paused) = paused {
                job.paused = paused;
            }
            if let Some(mirror) = mirror_deletions {
                job.mirror_deletions = job.kind == JobKind::Backup && mirror;
            }
            job.clone()
        };
        s.save_settings()?;
        Ok(view(&s, &job))
    })
    .await
}

#[tauri::command]
pub async fn remove_job(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        job(&s, &id)?;
        if locked(&s.jobs.running)?.contains(&id) {
            return Err("Wait until this backup or sync finishes, or pause it first.".into());
        }
        locked(&s.settings)?.jobs.retain(|j| j.id != id);
        s.save_settings()?;
        forget_index(&s, &id);
        s.log("Removed a backup or sync. No files were deleted.", "info");
        Ok(())
    })
    .await
}

/// Forgets what a removed job synced; files on both sides stay where they are.
fn forget_index(s: &Inner, id: &str) {
    let _ = folders::IndexStore::remove(&VaultIndexStore { vault: &s.vault }, id);
    let _ = folders::IndexStore::remove(&JsonIndexStore { dir: index_dir(s) }, id);
    if let Ok(mut labels) = locked(&s.jobs.labels) {
        labels.remove(id);
    }
    if let Ok(mut status) = locked(&s.jobs.status) {
        status.remove(id);
    }
}

#[tauri::command]
pub async fn run_job(state: State<'_, AppState>, id: String) -> AppResult<()> {
    job(&state.0, &id)?;
    spawn_run(state.0.clone(), id, RunMode::Normal);
    Ok(())
}

#[tauri::command]
pub async fn resolve_guard(state: State<'_, AppState>, id: String, apply: bool) -> AppResult<()> {
    job(&state.0, &id)?;
    spawn_run(
        state.0.clone(),
        id,
        if apply {
            RunMode::AllowDeletes
        } else {
            RunMode::Restore
        },
    );
    Ok(())
}

#[tauri::command]
pub async fn open_recycle(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        job(&s, &id)?;
        let path = recycle_root(&s).join(&id);
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        open::that_detached(&path).map_err(|e| e.to_string())
    })
    .await
}

#[derive(Clone, Copy)]
enum RunMode {
    Normal,
    AllowDeletes,
    Restore,
}

fn spawn_run(s: Arc<Inner>, id: String, mode: RunMode) {
    std::thread::spawn(move || {
        if let Err(error) = run_now(&s, &id, mode) {
            s.log(&error, "error");
        }
    });
}

struct Running<'a> {
    s: &'a Inner,
    id: String,
}
impl Drop for Running<'_> {
    fn drop(&mut self) {
        if let Ok(mut running) = self.s.jobs.running.lock() {
            running.remove(&self.id);
        }
    }
}

fn run_now(s: &Arc<Inner>, id: &str, mode: RunMode) -> AppResult<()> {
    if !locked(&s.jobs.running)?.insert(id.to_string()) {
        return Ok(());
    }
    let _running = Running { s, id: id.into() };
    locked(&s.jobs.last_attempt)?.insert(id.into(), now());
    let job = job(s, id)?;
    if job.paused {
        return Ok(());
    }
    set_status(s, id, JobStatus::new(JobState::Scanning));
    let result = execute(s, &job, mode);
    let status = match &result {
        Ok(outcome) => JobStatus::from_outcome(outcome),
        Err((state, message)) => JobStatus {
            state: *state,
            message: Some(message.clone()),
            pending_deletes: 0,
            file_count: 0,
        },
    };
    set_status(s, id, status);
    if let Ok(outcome) = &result {
        {
            let mut settings = locked(&s.settings)?;
            if let Some(job) = settings.jobs.iter_mut().find(|j| j.id == id) {
                job.last_run = Some(now());
            }
        }
        s.save_settings()?;
        let changed = outcome.uploaded + outcome.downloaded + outcome.trashed + outcome.recycled;
        if changed > 0 || outcome.conflicts > 0 {
            s.log(&summary(&job, outcome), "success");
        }
        if outcome.pending_deletes > 0 {
            s.log(
                &format!(
                    "{} files would be deleted by a {}. Review this on the {} page.",
                    outcome.pending_deletes,
                    kind_name(&job),
                    if job.kind == JobKind::Sync {
                        "Sync"
                    } else {
                        "Back up"
                    }
                ),
                "error",
            );
        }
        if job.location == TargetLocation::Vault && outcome.uploaded + outcome.trashed > 0 {
            if let Err(error) = crate::replication::sync(s) {
                s.log(&error, "error");
            }
        }
    }
    Ok(())
}

fn kind_name(job: &FolderJob) -> &'static str {
    match job.kind {
        JobKind::Sync => "sync",
        JobKind::Backup => "backup",
    }
}

fn summary(job: &FolderJob, outcome: &RunOutcome) -> String {
    let mut parts = Vec::new();
    for (count, text) in [
        (outcome.uploaded, "uploaded"),
        (outcome.downloaded, "downloaded"),
        (outcome.trashed, "moved to the trash"),
        (outcome.recycled, "moved to the recycle folder"),
        (outcome.conflicts, "kept as conflict copies"),
    ] {
        if count > 0 {
            parts.push(format!("{count} {text}"));
        }
    }
    let name = job
        .local_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!(
        "{} of {name}: {}.",
        if job.kind == JobKind::Sync {
            "Sync"
        } else {
            "Backup"
        },
        parts.join(", ")
    )
}

fn execute(
    s: &Arc<Inner>,
    job: &FolderJob,
    mode: RunMode,
) -> Result<RunOutcome, (JobState, String)> {
    let vault_job = job.location == TargetLocation::Vault;
    let mut trackers: HashMap<String, String> = HashMap::new();
    let mut progress = |p: folders::Progress| {
        // The engine reports each file as it starts; close the previous one.
        for (_, tracker) in trackers.drain() {
            s.transfers.finish(&tracker, &Ok(()));
        }
        let direction = match p.direction {
            folders::Direction::Upload => Direction::Upload,
            folders::Direction::Download => Direction::Download,
        };
        let tracker = s.transfers.start(&p.name, direction, vault_job, 0);
        trackers.insert(p.name, tracker);
    };
    let recycle = recycle_root(s);
    let result = if vault_job {
        if locked(&s.vault)
            .map_err(|e| (JobState::Error, e))?
            .is_none()
        {
            return Err((
                JobState::WaitingForVault,
                "Unlock the Vault to continue.".into(),
            ));
        }
        // Download missing encrypted chunks for the job folder first, without holding the
        // vault lock, and keep them from being evicted while the job runs.
        let _in_use = match &job.folder_id {
            Some(folder)
                if job.kind == JobKind::Sync && locked(&s.account).is_ok_and(|a| a.is_some()) =>
            {
                crate::replication::ensure_file(s, folder).ok()
            }
            _ => None,
        };
        let ensure = |_: &str| Ok(());
        let mut target = VaultTarget::new(&s.vault, job.folder_id.clone(), &ensure);
        let store = VaultIndexStore { vault: &s.vault };
        set_status(s, &job.id, JobStatus::new(JobState::Transferring));
        match mode {
            RunMode::Restore => {
                folders::run_restore(job, &mut target, &store, &recycle, &mut progress, now())
            }
            RunMode::Normal | RunMode::AllowDeletes => folders::run(
                job,
                &mut target,
                &store,
                &recycle,
                matches!(mode, RunMode::AllowDeletes),
                &mut progress,
                now(),
            ),
        }
    } else {
        let client = s.client().map_err(|_| {
            (
                JobState::WaitingForConnection,
                "Connect pCloud to continue.".to_string(),
            )
        })?;
        let root =
            folders::cloud_folder_id(job.folder_id.as_deref()).map_err(|e| (JobState::Error, e))?;
        if root == 0 {
            return Err((
                JobState::Error,
                "Choose a pCloud folder for this job instead of the whole pCloud.".into(),
            ));
        }
        cloud_safe(s, &client, root, true).map_err(|e| (JobState::Error, e))?;
        let mut target = CloudTarget::new(&client, root);
        let store = JsonIndexStore { dir: index_dir(s) };
        set_status(s, &job.id, JobStatus::new(JobState::Transferring));
        match mode {
            RunMode::Restore => {
                folders::run_restore(job, &mut target, &store, &recycle, &mut progress, now())
            }
            RunMode::Normal | RunMode::AllowDeletes => folders::run(
                job,
                &mut target,
                &store,
                &recycle,
                matches!(mode, RunMode::AllowDeletes),
                &mut progress,
                now(),
            ),
        }
    };
    for (_, tracker) in trackers.drain() {
        s.transfers
            .finish(&tracker, &result.as_ref().map(|_| ()).map_err(Clone::clone));
    }
    result.map_err(|error| {
        if error == folders::LOCAL_FOLDER_UNAVAILABLE {
            (JobState::WaitingForConnection, error)
        } else if error.contains("vault is locked") || error.contains("The vault is locked") {
            (
                JobState::WaitingForVault,
                "Unlock the Vault to continue.".into(),
            )
        } else {
            (JobState::Error, error)
        }
    })
}

/// Called from the background loop: runs every job that is due.
pub(crate) fn tick(s: &Arc<Inner>, app: &tauri::AppHandle) {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    let interval = if visible {
        VISIBLE_INTERVAL_MS
    } else {
        BACKGROUND_INTERVAL_MS
    };
    let Ok(jobs) = locked(&s.settings).map(|s| s.jobs.clone()) else {
        return;
    };
    for job in jobs.into_iter().filter(|j| !j.paused) {
        let waiting = locked(&s.jobs.status)
            .ok()
            .and_then(|m| m.get(&job.id).map(|st| st.state == JobState::NeedsReview))
            .unwrap_or(false);
        if waiting {
            continue; // the user decides first
        }
        let due = locked(&s.jobs.last_attempt)
            .ok()
            .and_then(|m| m.get(&job.id).copied())
            .is_none_or(|last| now().saturating_sub(last) >= interval);
        if due {
            if let Err(error) = run_now(s, &job.id, RunMode::Normal) {
                s.log(&error, "error");
            }
        }
    }
    let _ = folders::purge_recycle(&recycle_root(s), now());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Settings;
    use std::{collections::VecDeque, fs, sync::atomic::AtomicBool};

    fn inner() -> (tempfile::TempDir, Arc<Inner>) {
        let directory = tempfile::tempdir().unwrap();
        let inner = Arc::new(Inner {
            directory: directory.path().join("data"),
            vault: Arc::new(Mutex::new(None)),
            account: Mutex::new(None),
            settings: Mutex::new(Settings::default()),
            activity: Mutex::new(VecDeque::new()),
            syncing: AtomicBool::new(false),
            in_use: Mutex::default(),
            transfers: Default::default(),
            jobs: Default::default(),
        });
        fs::create_dir_all(&inner.directory).unwrap();
        *inner.vault.lock().unwrap() = Some(
            vault_core::Vault::create(&inner.vault_path(), "test-only-long-passphrase").unwrap(),
        );
        (directory, inner)
    }

    #[test]
    fn vault_sync_job_encrypts_files_and_reports_status() {
        let (directory, s) = inner();
        let local = directory.path().join("Documents");
        fs::create_dir_all(local.join("Letters")).unwrap();
        fs::write(local.join("Letters/hello.txt"), b"private-sync-marker").unwrap();
        let folder = resolve_destination(&s, TargetLocation::Vault, None, &local).unwrap();
        let job = FolderJob {
            id: folders::new_job_id(),
            kind: JobKind::Sync,
            local_path: local.clone(),
            location: TargetLocation::Vault,
            folder_id: folder,
            paused: false,
            mirror_deletions: false,
            last_run: None,
        };
        s.settings.lock().unwrap().jobs.push(job.clone());
        run_now(&s, &job.id, RunMode::Normal).unwrap();
        let view = view(&s, &job);
        assert_eq!(view.state, JobState::Idle, "{:?}", view.message);
        assert_eq!(view.target.label, "Vault / Documents");
        let vault = s.vault.lock().unwrap();
        let vault = vault.as_ref().unwrap();
        let file = vault
            .all_entries()
            .into_iter()
            .find(|e| e.name == "hello.txt")
            .expect("file synced into the vault");
        assert_eq!(file.size, 19);
        // Nothing readable on disk: neither the objects nor the encrypted sync index.
        for entry in walk(&s.vault_path()) {
            let bytes = fs::read(entry).unwrap();
            assert!(!bytes.windows(19).any(|w| w == b"private-sync-marker"));
            assert!(!bytes.windows(9).any(|w| w == b"hello.txt"));
        }
        assert!(s
            .transfers
            .list()
            .iter()
            .any(|t| t.name.ends_with("hello.txt")));
    }

    #[test]
    fn jobs_wait_for_the_vault_and_for_pcloud() {
        let (directory, s) = inner();
        let local = directory.path().join("Pictures");
        fs::create_dir_all(&local).unwrap();
        let mut job = FolderJob {
            id: folders::new_job_id(),
            kind: JobKind::Backup,
            local_path: local,
            location: TargetLocation::Cloud,
            folder_id: Some("d5".into()),
            paused: false,
            mirror_deletions: false,
            last_run: None,
        };
        s.settings.lock().unwrap().jobs.push(job.clone());
        run_now(&s, &job.id, RunMode::Normal).unwrap();
        assert_eq!(view(&s, &job).state, JobState::WaitingForConnection);
        job.id = folders::new_job_id();
        job.location = TargetLocation::Vault;
        job.folder_id = None;
        s.settings.lock().unwrap().jobs.push(job.clone());
        *s.vault.lock().unwrap() = None;
        run_now(&s, &job.id, RunMode::Normal).unwrap();
        assert_eq!(view(&s, &job).state, JobState::WaitingForVault);
    }

    fn walk(path: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        for entry in fs::read_dir(path).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else if path.file_name().is_some_and(|n| n != "vault.lock") {
                files.push(path);
            }
        }
        files
    }
}
