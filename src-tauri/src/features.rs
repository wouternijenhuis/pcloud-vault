//! Commands for the 0.3 sections: preferences, account, uploads to pCloud, photos,
//! public links, shared folders and file requests.
//!
//! Encryption boundary: anything addressed with `location == "cloud"` is plain pCloud data
//! that pCloud can read. Links, shares and file requests only ever touch plain pCloud
//! folders; vault entries are refused with an explanation.

use crate::app::{
    cloud_id, cloud_safe, import_cloud_into_vault, import_local, locked, now, outside_storage,
    preview_data, task, AppResult, AppState, Inner,
};
use crate::transfers::Direction;
use base64::Engine;
use pcloud_api::Client;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{Manager, State};

pub const PROTECTIONS: [&str; 3] = ["everything", "choose", "vaultOnly"];
const THEMES: [&str; 3] = ["system", "light", "dark"];
const VAULT_SHARE_REFUSAL: &str = "Vault files can't be shared: sharing would let pCloud read them. Export a copy and upload it to pCloud Files to share it.";

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestRule {
    pub link_id: u64,
    /// pCloud folder that receives uploads.
    pub folder_id: u64,
    /// Vault folder that received files are moved into (None = Vault root).
    pub vault_folder_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    protection: Option<String>,
    theme: String,
    start_at_login: bool,
}

fn preferences(s: &Inner) -> AppResult<Preferences> {
    let settings = locked(&s.settings)?;
    Ok(Preferences {
        protection: settings.protection.clone(),
        theme: settings.theme.clone(),
        start_at_login: settings.start_at_login,
    })
}

/// The protection mode decides where new uploads, backups and syncs may go.
pub(crate) fn destination_allowed(s: &Inner, vault: bool) -> AppResult<()> {
    match locked(&s.settings)?.protection.as_deref() {
        Some("everything") if !vault => Err("Your settings keep everything encrypted, so this can only go to the Vault. Change this in Settings → Protection.".into()),
        Some("vaultOnly") if vault => Err("Your settings send uploads, backups and syncs to pCloud without encryption. Use the Vault page to add encrypted files, or change this in Settings → Protection.".into()),
        _ => Ok(()),
    }
}

#[tauri::command]
pub async fn get_preferences(state: State<'_, AppState>) -> AppResult<Preferences> {
    task(&state, move |s| preferences(&s)).await
}

#[tauri::command]
pub async fn set_preferences(
    state: State<'_, AppState>,
    protection: Option<String>,
    theme: Option<String>,
    start_at_login: Option<bool>,
) -> AppResult<Preferences> {
    task(&state, move |s| {
        {
            let mut settings = locked(&s.settings)?;
            if let Some(protection) = protection {
                if !PROTECTIONS.contains(&protection.as_str()) {
                    return Err("Unknown protection mode".into());
                }
                settings.protection = Some(protection);
            }
            if let Some(theme) = theme {
                if !THEMES.contains(&theme.as_str()) {
                    return Err("Unknown theme".into());
                }
                settings.theme = theme;
            }
            if let Some(start) = start_at_login {
                settings.start_at_login = start;
            }
        }
        s.save_settings()?;
        if start_at_login.is_some() {
            let remembered = s.directory.join("device.key").exists();
            let start = locked(&s.settings)?.start_at_login;
            crate::app::set_startup(remembered && start)?;
        }
        preferences(&s)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    email: String,
    region: String,
    used_bytes: u64,
    quota_bytes: u64,
    premium: bool,
}

#[tauri::command]
pub async fn account_info(state: State<'_, AppState>) -> AppResult<AccountInfo> {
    task(&state, move |s| {
        let client = s.client()?;
        let info = client.user_info().map_err(|e| e.to_string())?;
        Ok(AccountInfo {
            email: info.email,
            region: client.account().region.as_str().into(),
            used_bytes: info.used_quota,
            quota_bytes: info.quota,
            premium: info.premium,
        })
    })
    .await
}

#[derive(Serialize)]
pub struct KnownFolders {
    documents: Option<String>,
    pictures: Option<String>,
    desktop: Option<String>,
}

#[tauri::command]
pub async fn known_folders(app: tauri::AppHandle) -> AppResult<KnownFolders> {
    let path = app.path();
    let existing = |p: tauri::Result<PathBuf>| {
        p.ok()
            .filter(|p| p.is_dir())
            .map(|p| p.to_string_lossy().into_owned())
    };
    Ok(KnownFolders {
        documents: existing(path.document_dir()),
        pictures: existing(path.picture_dir()),
        desktop: existing(path.desktop_dir()),
    })
}

#[tauri::command]
pub async fn pick_folder(state: State<'_, AppState>) -> AppResult<Option<String>> {
    task(&state, move |_| {
        Ok(rfd::FileDialog::new()
            .set_title("Choose a folder on this computer")
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned()))
    })
    .await
}

/// Only opens https pages of this project or pCloud; never arbitrary schemes or hosts.
pub(crate) fn allowed_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.contains(['@', ':', '\\']) || url.chars().any(char::is_control) {
        return false;
    }
    let project = host == "github.com"
        && (rest == "github.com/wouternijenhuis/pcloud-vault"
            || rest.starts_with("github.com/wouternijenhuis/pcloud-vault/")
            || rest.starts_with("github.com/wouternijenhuis/pcloud-vault?")
            || rest.starts_with("github.com/wouternijenhuis/pcloud-vault#"));
    let pcloud = ["pcloud.com", "pcloud.link"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    project || pcloud
}

#[tauri::command]
pub async fn open_url(state: State<'_, AppState>, url: String) -> AppResult<()> {
    task(&state, move |_| {
        if !allowed_url(&url) {
            return Err("This link can't be opened from the app.".into());
        }
        open::that_detached(&url).map_err(|e| e.to_string())
    })
    .await
}

// ---------- Uploads ----------

/// Uploads local files or folders into a plain pCloud folder. Existing names are never
/// overwritten: pCloud renames the new copy instead.
fn upload_to_cloud(s: &Arc<Inner>, paths: Vec<PathBuf>, folder: u64) -> AppResult<()> {
    let client = s.client()?;
    cloud_safe(s, &client, folder, true)?;
    let storage = fs::canonicalize(&s.directory).map_err(|e| e.to_string())?;
    for path in paths {
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        outside_storage(s, &path)?;
        if crate::app::path_key(&storage).starts_with(crate::app::path_key(&path)) {
            return Err(
                "This folder contains the app's own storage. Choose a narrower folder.".into(),
            );
        }
        upload_walk(s, &client, &path, folder, 0)?;
    }
    s.log(
        "Upload finished. These files are stored in pCloud without encryption.",
        "success",
    );
    Ok(())
}

fn upload_walk(
    s: &Arc<Inner>,
    client: &Client,
    path: &Path,
    parent: u64,
    depth: usize,
) -> AppResult<()> {
    if depth > 128 {
        return Err("Folder upload exceeds the supported depth.".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid filename")?;
    if metadata.is_dir() {
        let existing = client
            .list_folder(parent)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|m| m.is_folder && m.name == name);
        let folder = match existing {
            Some(folder) => folder,
            None => client
                .create_folder(parent, name)
                .map_err(|e| e.to_string())?,
        };
        for child in fs::read_dir(path).map_err(|e| e.to_string())? {
            upload_walk(
                s,
                client,
                &child.map_err(|e| e.to_string())?.path(),
                folder.id,
                depth + 1,
            )?;
        }
        Ok(())
    } else if metadata.is_file() {
        let total = metadata.len();
        let tracker = s.transfers.start(name, Direction::Upload, false, total);
        let reader = Progress {
            inner: File::open(path).map_err(|e| e.to_string())?,
            state: s.clone(),
            tracker: tracker.clone(),
            done: 0,
        };
        let result = client
            .upload_reader(parent, name, reader, total, true)
            .map(|_| ())
            .map_err(|e| e.to_string());
        s.transfers.finish(&tracker, &result);
        result
    } else {
        Ok(())
    }
}

struct Progress {
    inner: File,
    state: Arc<Inner>,
    tracker: String,
    done: u64,
}
impl Read for Progress {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.done += count as u64;
        self.state.transfers.progress(&self.tracker, self.done);
        Ok(count)
    }
}

fn upload(
    s: &Arc<Inner>,
    paths: Vec<PathBuf>,
    location: &str,
    folder_id: Option<String>,
) -> AppResult<()> {
    if paths.len() > 1024 {
        return Err("Upload at most 1024 top-level items at a time.".into());
    }
    match location {
        "vault" => {
            destination_allowed(s, true)?;
            import_local(s, paths, folder_id)
        }
        "cloud" => {
            destination_allowed(s, false)?;
            let folder = match folder_id {
                Some(id) => match cloud_id(&id)? {
                    (id, true) => id,
                    _ => return Err("Choose a pCloud folder as the destination.".into()),
                },
                None => 0,
            };
            upload_to_cloud(s, paths, folder)
        }
        _ => Err("Unknown storage location".into()),
    }
}

#[tauri::command]
pub async fn upload_paths(
    state: State<'_, AppState>,
    paths: Vec<String>,
    location: String,
    folder_id: Option<String>,
) -> AppResult<()> {
    task(&state, move |s| {
        upload(
            &s,
            paths.into_iter().map(PathBuf::from).collect(),
            &location,
            folder_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn upload_files(
    state: State<'_, AppState>,
    location: String,
    folder_id: Option<String>,
) -> AppResult<()> {
    task(&state, move |s| {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Choose files to upload")
            .pick_files()
        else {
            return Ok(());
        };
        upload(&s, paths, &location, folder_id)
    })
    .await
}

#[tauri::command]
pub async fn upload_folder(
    state: State<'_, AppState>,
    location: String,
    folder_id: Option<String>,
) -> AppResult<()> {
    task(&state, move |s| {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Choose a folder to upload")
            .pick_folder()
        else {
            return Ok(());
        };
        upload(&s, vec![path], &location, folder_id)
    })
    .await
}

#[tauri::command]
pub async fn get_transfers(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::transfers::Transfer>> {
    task(&state, move |s| Ok(s.transfers.list())).await
}

#[tauri::command]
pub async fn clear_transfers(state: State<'_, AppState>) -> AppResult<()> {
    task(&state, move |s| {
        s.transfers.clear_finished();
        Ok(())
    })
    .await
}

// ---------- Photos ----------

const IMAGE_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "webp", "heic"];
const MAX_PHOTOS: usize = 5000;
const CLOUD_PHOTO_TTL_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    id: String,
    name: String,
    location: &'static str,
    modified: u64,
    size: u64,
}

pub(crate) fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

static CLOUD_PHOTOS: Mutex<Option<(u64, Vec<Photo>)>> = Mutex::new(None);

fn cloud_photos(s: &Inner) -> AppResult<Vec<Photo>> {
    if let Some((time, photos)) = locked(&CLOUD_PHOTOS)?.as_ref() {
        if now().saturating_sub(*time) < CLOUD_PHOTO_TTL_MS {
            return Ok(photos.clone());
        }
    }
    let client = s.client()?;
    let reserved = locked(&s.settings)?.remote_folder;
    let tree = client.list_tree(0).map_err(|e| e.to_string())?;
    // Exclude everything below the vault's own storage folder.
    let mut excluded = std::collections::BTreeSet::new();
    if let Some(reserved) = reserved {
        excluded.insert(reserved);
        let mut changed = true;
        while changed {
            changed = false;
            for item in tree.iter().filter(|m| m.is_folder) {
                if excluded.contains(&item.parent_folder_id) && excluded.insert(item.id) {
                    changed = true;
                }
            }
        }
    }
    let photos: Vec<Photo> = tree
        .into_iter()
        .filter(|m| !m.is_folder && is_image(&m.name) && !excluded.contains(&m.parent_folder_id))
        .map(|m| Photo {
            id: format!("f{}", m.id),
            name: m.name,
            location: "cloud",
            modified: m.modified * 1000,
            size: m.size,
        })
        .collect();
    *locked(&CLOUD_PHOTOS)? = Some((now(), photos.clone()));
    Ok(photos)
}

#[tauri::command]
pub async fn list_photos(state: State<'_, AppState>, source: String) -> AppResult<Vec<Photo>> {
    task(&state, move |s| {
        let mut photos = Vec::new();
        if source == "all" || source == "vault" {
            if let Some(vault) = locked(&s.vault)?.as_ref() {
                photos.extend(
                    vault
                        .all_entries()
                        .into_iter()
                        .filter(|e| {
                            e.kind == vault_core::EntryKind::File && !e.trashed && is_image(&e.name)
                        })
                        .map(|e| Photo {
                            id: e.id,
                            name: e.name,
                            location: "vault",
                            modified: e.modified,
                            size: e.size,
                        }),
                );
            }
        }
        if (source == "all" || source == "cloud") && locked(&s.account)?.is_some() {
            match cloud_photos(&s) {
                Ok(cloud) => photos.extend(cloud),
                Err(error) if source == "cloud" => return Err(error),
                Err(_) => {}
            }
        }
        photos.sort_by_key(|p| std::cmp::Reverse(p.modified));
        photos.truncate(MAX_PHOTOS);
        Ok(photos)
    })
    .await
}

/// Thumbnail as a data URL. Vault images are decrypted in memory and returned whole; the
/// UI downsizes them and keeps them only in memory, so no plaintext thumbnail hits disk.
#[tauri::command]
pub async fn photo_thumb(
    state: State<'_, AppState>,
    location: String,
    id: String,
) -> AppResult<Option<String>> {
    task(&state, move |s| {
        if location != "cloud" {
            return preview_data(&s, &location, &id);
        }
        let client = s.client()?;
        let (number, folder) = cloud_id(&id)?;
        if folder {
            return Ok(None);
        }
        cloud_safe(&s, &client, number, false)?;
        let bytes = client
            .thumbnail(number, "256x256")
            .map_err(|e| e.to_string())?;
        let mime = if bytes.starts_with(b"\x89PNG") {
            "image/png"
        } else {
            "image/jpeg"
        };
        Ok(Some(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        )))
    })
    .await
}

// ---------- Public links ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    id: String,
    entry_id: String,
    name: String,
    url: String,
    created: u64,
    downloads: u64,
    is_folder: bool,
}

fn link_view(link: pcloud_api::PublicLink) -> LinkView {
    LinkView {
        id: link.link_id.to_string(),
        entry_id: match (link.file_id, link.folder_id) {
            (Some(file), _) => format!("f{file}"),
            (None, Some(folder)) => format!("d{folder}"),
            _ => String::new(),
        },
        name: link.name,
        url: link.url,
        created: parse_pcloud_time(&link.created).unwrap_or(0),
        downloads: link.downloads,
        is_folder: link.is_folder,
    }
}

/// Parses pCloud's RFC 2822 style times ("Tue, 18 Nov 2026 10:00:00 +0000") or a unix
/// timestamp into milliseconds.
pub(crate) fn parse_pcloud_time(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(seconds * 1000);
    }
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [_, day, month, year, time, ..] = parts.as_slice() else {
        return None;
    };
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m == month)? as i64
        + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let clock: Vec<i64> = time
        .split(':')
        .map(|x| x.parse().ok())
        .collect::<Option<_>>()?;
    let [hour, minute, second] = clock.as_slice() else {
        return None;
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86400 + hour * 3600 + minute * 60 + second;
    u64::try_from(seconds).ok().map(|s| s * 1000)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// UTC "YYYY-MM-DD HH:MM:SS" for pCloud datetime parameters.
pub(crate) fn format_pcloud_time(ms: u64) -> String {
    let seconds = (ms / 1000) as i64;
    let days = seconds.div_euclid(86400);
    let rest = seconds.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn plain_cloud_entry(s: &Inner, client: &Client, id: &str) -> AppResult<(u64, bool)> {
    if !(id.starts_with('f') || id.starts_with('d')) {
        return Err(VAULT_SHARE_REFUSAL.into());
    }
    let (number, folder) = cloud_id(id).map_err(|_| VAULT_SHARE_REFUSAL.to_string())?;
    cloud_safe(s, client, number, folder)?;
    Ok((number, folder))
}

#[tauri::command]
pub async fn list_links(state: State<'_, AppState>) -> AppResult<Vec<LinkView>> {
    task(&state, move |s| {
        Ok(s.client()?
            .list_links()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(link_view)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn create_link(state: State<'_, AppState>, id: String) -> AppResult<LinkView> {
    task(&state, move |s| {
        let client = s.client()?;
        let (number, folder) = plain_cloud_entry(&s, &client, &id)?;
        let link = if folder {
            client.create_folder_link(number)
        } else {
            client.create_file_link(number)
        }
        .map_err(|e| e.to_string())?;
        s.log(
            "Created a public link. Anyone with the link can open the item.",
            "info",
        );
        Ok(link_view(link))
    })
    .await
}

#[tauri::command]
pub async fn delete_link(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        let id: u64 = id.parse().map_err(|_| "Invalid link")?;
        s.client()?.delete_link(id).map_err(|e| e.to_string())
    })
    .await
}

// ---------- Shared folders ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareView {
    id: String,
    direction: &'static str,
    folder_name: String,
    email: String,
    permissions: &'static str,
    state: &'static str,
    folder_id: Option<String>,
}

#[tauri::command]
pub async fn list_shares(state: State<'_, AppState>) -> AppResult<Vec<ShareView>> {
    task(&state, move |s| {
        Ok(s.client()?
            .list_shares()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|share| ShareView {
                id: format!("{}{}", if share.is_request { 'r' } else { 's' }, share.id),
                direction: if share.incoming {
                    "incoming"
                } else {
                    "outgoing"
                },
                folder_name: share.folder_name,
                email: share.email,
                permissions: if share.can_modify { "edit" } else { "view" },
                state: if share.is_request {
                    "pending"
                } else {
                    "active"
                },
                folder_id: share.folder_id.map(|id| format!("d{id}")),
            })
            .collect())
    })
    .await
}

fn share_id(id: &str) -> AppResult<(bool, u64)> {
    let (kind, number) = id.split_at_checked(1).ok_or("Invalid share")?;
    let number = number.parse().map_err(|_| "Invalid share")?;
    match kind {
        "r" => Ok((true, number)),
        "s" => Ok((false, number)),
        _ => Err("Invalid share".into()),
    }
}

#[tauri::command]
pub async fn share_folder(
    state: State<'_, AppState>,
    id: String,
    email: String,
    permissions: String,
) -> AppResult<()> {
    task(&state, move |s| {
        let client = s.client()?;
        let (number, folder) = plain_cloud_entry(&s, &client, &id)?;
        if !folder {
            return Err("Only folders can be shared with other pCloud users. Create a link to share a file.".into());
        }
        client
            .share_folder(number, email.trim(), permissions == "edit")
            .map_err(|e| e.to_string())?;
        s.log("Sent a folder share invitation through pCloud.", "info");
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn accept_share(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| match share_id(&id)? {
        (true, number) => s.client()?.accept_share(number).map_err(|e| e.to_string()),
        _ => Err("This share was already accepted.".into()),
    })
    .await
}

#[tauri::command]
pub async fn decline_share(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| match share_id(&id)? {
        (true, number) => s.client()?.decline_share(number).map_err(|e| e.to_string()),
        _ => Err("Use Stop sharing for an accepted share.".into()),
    })
    .await
}

#[tauri::command]
pub async fn remove_share(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        let client = s.client()?;
        match share_id(&id)? {
            (true, number) => client.cancel_share_request(number),
            (false, number) => client.remove_share(number),
        }
        .map_err(|e| e.to_string())
    })
    .await
}

// ---------- File requests ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRequestView {
    id: String,
    folder_id: String,
    folder_name: String,
    url: String,
    comment: String,
    expires: Option<u64>,
    max_files: Option<u64>,
    files_received: u64,
    encrypt: bool,
    vault_folder_id: Option<String>,
    vault_folder_name: Option<String>,
}

fn request_view(
    s: &Inner,
    client: &Client,
    link: pcloud_api::UploadLink,
) -> AppResult<FileRequestView> {
    let rule = locked(&s.settings)?
        .file_requests
        .iter()
        .find(|r| r.link_id == link.id)
        .cloned();
    let folder_name = client
        .stat(link.folder_id, true)
        .map(|m| m.name)
        .unwrap_or_else(|_| "pCloud folder".into());
    let vault_folder_name = match &rule {
        Some(RequestRule {
            vault_folder_id: Some(id),
            ..
        }) => locked(&s.vault)?
            .as_ref()
            .and_then(|v| v.entry(id).ok())
            .map(|e| e.name),
        Some(_) => Some("Vault".into()),
        None => None,
    };
    Ok(FileRequestView {
        id: link.id.to_string(),
        folder_id: format!("d{}", link.folder_id),
        folder_name,
        url: link.url,
        comment: link.comment,
        expires: link.expires.as_deref().and_then(parse_pcloud_time),
        max_files: link.max_files,
        files_received: link.files,
        encrypt: rule.is_some(),
        vault_folder_id: rule.and_then(|r| r.vault_folder_id),
        vault_folder_name,
    })
}

#[tauri::command]
pub async fn list_file_requests(state: State<'_, AppState>) -> AppResult<Vec<FileRequestView>> {
    task(&state, move |s| {
        let client = s.client()?;
        client
            .list_upload_links()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|link| request_view(&s, &client, link))
            .collect()
    })
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_file_request(
    state: State<'_, AppState>,
    folder_id: String,
    comment: String,
    expires_days: Option<u64>,
    max_files: Option<u64>,
    encrypt: bool,
    vault_folder_id: Option<String>,
) -> AppResult<FileRequestView> {
    task(&state, move |s| {
        let client = s.client()?;
        let (folder, is_folder) = plain_cloud_entry(&s, &client, &folder_id)?;
        if !is_folder {
            return Err("Choose a pCloud folder to receive files.".into());
        }
        let encrypt = encrypt || locked(&s.settings)?.protection.as_deref() == Some("everything");
        if encrypt {
            let vault = locked(&s.vault)?;
            let vault = vault
                .as_ref()
                .ok_or("Unlock your Vault to move received files into it.")?;
            if let Some(id) = &vault_folder_id {
                let entry = vault.entry(id).map_err(|e| e.to_string())?;
                if entry.kind != vault_core::EntryKind::Folder || entry.trashed {
                    return Err("Choose a Vault folder for received files.".into());
                }
            }
        }
        let expires = match expires_days {
            Some(days) if (1..=3650).contains(&days) => {
                Some(format_pcloud_time(now() + days * 86_400_000))
            }
            Some(_) => return Err("Choose an expiry between 1 and 3650 days.".into()),
            None => None,
        };
        if max_files.is_some_and(|n| n == 0 || n > 100_000) {
            return Err("Choose a file limit between 1 and 100000.".into());
        }
        let link = client
            // pCloud requires a message; it is the only text people uploading see.
            .create_upload_link(
                folder,
                if comment.trim().is_empty() {
                    "Please upload your files here."
                } else {
                    comment.trim()
                },
                expires.as_deref(),
                max_files,
            )
            .map_err(|e| e.to_string())?;
        if encrypt {
            locked(&s.settings)?.file_requests.push(RequestRule {
                link_id: link.id,
                folder_id: folder,
                vault_folder_id,
            });
            s.save_settings()?;
        }
        s.log(
            "Created a file request. Received files arrive unencrypted on pCloud first.",
            "info",
        );
        request_view(&s, &client, link)
    })
    .await
}

#[tauri::command]
pub async fn delete_file_request(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        let id: u64 = id.parse().map_err(|_| "Invalid file request")?;
        s.client()?
            .delete_upload_link(id)
            .map_err(|e| e.to_string())?;
        locked(&s.settings)?
            .file_requests
            .retain(|r| r.link_id != id);
        s.save_settings()
    })
    .await
}

/// Moves files received through auto-encrypting file requests into the Vault. Each file is
/// imported and verified first; only then is the plain copy moved to the pCloud trash.
pub(crate) fn encrypt_received(s: &Arc<Inner>) -> AppResult<usize> {
    let rules = locked(&s.settings)?.file_requests.clone();
    if rules.is_empty() || locked(&s.vault)?.is_none() {
        return Ok(0);
    }
    let client = s.client()?;
    let mut moved = 0;
    for rule in rules {
        let files = match client.list_folder(rule.folder_id) {
            Ok(files) => files,
            Err(error) => {
                s.log(
                    &format!("A file request folder could not be read: {error}"),
                    "error",
                );
                continue;
            }
        };
        for file in files.into_iter().filter(|m| !m.is_folder) {
            if cloud_safe(s, &client, file.id, false).is_err() {
                continue;
            }
            let worker = s.client()?;
            match import_cloud_into_vault(s, worker, file.id, rule.vault_folder_id.as_deref()) {
                Ok(_) => {
                    client.delete_file(file.id).map_err(|e| e.to_string())?;
                    moved += 1;
                }
                Err(error) => s.log(
                    &format!("A received file stays unencrypted for now: {error}"),
                    "error",
                ),
            }
        }
    }
    if moved > 0 {
        s.log(&format!("Encrypted {moved} received file(s) into the Vault and moved the plain copies to the pCloud trash."), "success");
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_project_and_pcloud_https_links_open() {
        assert!(allowed_url(
            "https://github.com/wouternijenhuis/pcloud-vault/issues"
        ));
        assert!(allowed_url("https://u.pcloud.link/publink/show?code=XYZ"));
        assert!(allowed_url("https://my.pcloud.com/"));
        assert!(!allowed_url(
            "http://github.com/wouternijenhuis/pcloud-vault"
        ));
        assert!(!allowed_url("https://github.com/someone-else/repo"));
        assert!(!allowed_url(
            "https://github.com/wouternijenhuis/pcloud-vault-evil"
        ));
        assert!(!allowed_url("https://pcloud.com.example.com/"));
        assert!(!allowed_url("https://evilpcloud.com/"));
        assert!(!allowed_url("https://my.pcloud.com@example.com/"));
        assert!(!allowed_url("file:///etc/passwd"));
    }

    #[test]
    fn pcloud_times_roundtrip() {
        let ms = parse_pcloud_time("Wed, 18 Nov 2026 10:20:30 +0000").unwrap();
        assert_eq!(format_pcloud_time(ms), "2026-11-18 10:20:30");
        assert_eq!(parse_pcloud_time("1700000000"), Some(1_700_000_000_000));
        assert_eq!(format_pcloud_time(0), "1970-01-01 00:00:00");
        assert!(parse_pcloud_time("not a date").is_none());
    }

    #[test]
    fn images_are_recognised_by_extension() {
        assert!(is_image("Beach.JPG"));
        assert!(is_image("scan.webp"));
        assert!(!is_image("notes.txt"));
        assert!(!is_image("jpg"));
    }

    #[test]
    fn share_ids_distinguish_requests_and_shares() {
        assert_eq!(share_id("r12").unwrap(), (true, 12));
        assert_eq!(share_id("s7").unwrap(), (false, 7));
        assert!(share_id("x1").is_err());
        assert!(share_id("r").is_err());
    }
}
