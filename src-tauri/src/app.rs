use crate::replication;
use base64::Engine;
use pcloud_api::{Account, Client};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use vault_core::{Entry, EntryKind, Vault};
use zeroize::Zeroizing;

pub type AppResult<T> = Result<T, String>;
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Settings {
    pub cache_limit_gib: u64,
    pub client_id: String,
    pub remote_folder: Option<u64>,
    pub remote_objects: Option<u64>,
    /// pCloud change-feed position for incremental object discovery. Renamed from the
    /// former `diffCursor`, so the first sync after upgrading does a full listing.
    pub object_cursor: Option<u64>,
    pub last_sync: Option<u64>,
    /// Salts of password wrappers this computer has superseded; never adopted again.
    pub retired_header_salts: Vec<String>,
    /// "everything", "choose" or "vaultOnly"; None until the user chose (upgrade from 0.2).
    pub protection: Option<String>,
    /// "system", "light" or "dark".
    pub theme: String,
    /// Start in the background at sign-in while this computer is remembered.
    pub start_at_login: bool,
    pub jobs: Vec<crate::folders::FolderJob>,
    /// File requests whose received files are moved into the Vault.
    pub file_requests: Vec<crate::features::RequestRule>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            cache_limit_gib: 10,
            client_id: String::new(),
            remote_folder: None,
            remote_objects: None,
            object_cursor: None,
            last_sync: None,
            retired_header_salts: Vec::new(),
            protection: None,
            theme: "system".into(),
            start_at_login: true,
            jobs: Vec::new(),
            file_requests: Vec::new(),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    pub message: String,
    pub time: u64,
    pub kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    vault_exists: bool,
    unlocked: bool,
    connected: bool,
    account_email: Option<String>,
    region: Option<String>,
    cache_limit_bytes: u64,
    cache_bytes: u64,
    pending_count: usize,
    last_sync: Option<u64>,
    oauth_client_id: String,
    error: Option<String>,
}

pub(crate) struct Inner {
    pub directory: PathBuf,
    pub vault: Arc<Mutex<Option<Vault>>>,
    pub account: Mutex<Option<Account>>,
    pub settings: Mutex<Settings>,
    pub activity: Mutex<VecDeque<Activity>>,
    pub syncing: AtomicBool,
    /// Vault entries currently being read; their chunks are protected from eviction.
    pub in_use: Mutex<std::collections::HashMap<String, usize>>,
    pub transfers: crate::transfers::Transfers,
    pub jobs: crate::jobs::JobRuntime,
}
#[derive(Clone)]
pub struct AppState(pub(crate) Arc<Inner>);

pub(crate) fn locked<T>(mutex: &Mutex<T>) -> AppResult<std::sync::MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| {
        "The client encountered an internal error. Restart it to recover durable writes.".into()
    })
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path.parent().ok_or("Invalid local data path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = parent.join(format!(
        "write-{}-{}-{serial}.tmp",
        std::process::id(),
        now()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            },
        };
        let from: Vec<_> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    fs::rename(temp, path).map_err(|e| e.to_string())?;
    Ok(())
}
impl Inner {
    pub fn vault_path(&self) -> PathBuf {
        self.directory.join("vault")
    }
    pub fn log(&self, message: &str, kind: &str) {
        if let Ok(mut activity) = self.activity.lock() {
            let time = now();
            let id = format!("{time}-{}", activity.len());
            activity.push_front(Activity {
                id,
                message: message.into(),
                time,
                kind: kind.into(),
            });
            activity.truncate(250);
        }
    }
    pub fn save_settings(&self) -> AppResult<()> {
        let bytes =
            serde_json::to_vec_pretty(&*locked(&self.settings)?).map_err(|e| e.to_string())?;
        atomic_write(&self.directory.join("settings.json"), &bytes)
    }
    pub fn client(&self) -> AppResult<Client> {
        let account = locked(&self.account)?
            .clone()
            .ok_or("Connect your pCloud account first")?;
        Client::new(account).map_err(|e| e.to_string())
    }
    fn status(&self) -> AppResult<AppStatus> {
        let settings = locked(&self.settings)?.clone();
        let account = locked(&self.account)?;
        let (unlocked, pending) = {
            let vault = locked(&self.vault)?;
            (
                vault.is_some(),
                vault
                    .as_ref()
                    .map(|v| v.pending_object_ids().len())
                    .unwrap_or(0),
            )
        };
        let bytes = fs::read_dir(self.vault_path().join("objects"))
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|f| f.metadata().ok())
            .map(|m| m.len())
            .sum();
        Ok(AppStatus {
            vault_exists: self.vault_path().join("vault.json").exists(),
            unlocked,
            connected: account.is_some(),
            account_email: account.as_ref().map(|a| a.email.clone()),
            region: account.as_ref().map(|a| a.region.as_str().into()),
            cache_limit_bytes: settings.cache_limit_gib * 1024 * 1024 * 1024,
            cache_bytes: bytes,
            pending_count: pending,
            last_sync: settings.last_sync,
            oauth_client_id: settings.client_id,
            error: None,
        })
    }
    /// Remembering is a convenience: when it fails (for example no keyring on Linux),
    /// the vault still opens for this session and the reason is shown in Activity.
    fn remember_or_warn(&self, vault: &Vault, remember: bool) {
        if let Err(error) = self.remember(vault, remember) {
            self.log(
                &format!("This computer could not be remembered: {error}"),
                "error",
            );
        }
    }
    fn remember(&self, vault: &Vault, remember: bool) -> AppResult<()> {
        let path = self.directory.join("device.key");
        if remember {
            let bytes = Zeroizing::new(
                serde_json::to_vec(&(vault.vault_id(), vault.key_bytes()))
                    .map_err(|e| e.to_string())?,
            );
            let protected =
                pcloud_api::device_protection::protect(&bytes).map_err(|e| e.to_string())?;
            atomic_write(&path, &protected)?;
        } else if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        set_startup(remember && locked(&self.settings)?.start_at_login)?;
        Ok(())
    }
}

/// Starts the app in the background at sign-in while a device is remembered.
pub(crate) fn set_startup(enabled: bool) -> AppResult<()> {
    if std::env::args().any(|arg| arg == "--data-dir") {
        return Ok(());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    set_platform_startup(enabled, &executable)
}

#[cfg(windows)]
fn set_platform_startup(enabled: bool, executable: &Path) -> AppResult<()> {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?
        .0;
    if enabled {
        key.set_value(
            "pCloudVault",
            &format!("\"{}\" --background", executable.display()),
        )
        .map_err(|e| e.to_string())?;
    } else {
        match key.delete_value("pCloudVault") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

/// Linux: an XDG autostart entry. macOS: a per-user LaunchAgent.
#[cfg(not(windows))]
fn set_platform_startup(enabled: bool, executable: &Path) -> AppResult<()> {
    let (path, contents) = startup_entry(executable)?;
    if enabled {
        atomic_write(&path, contents.as_bytes())
    } else {
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
fn startup_entry(executable: &Path) -> AppResult<(PathBuf, String)> {
    let location = executable.to_string_lossy();
    if location.contains("/AppTranslocation/") || location.starts_with("/Volumes/") {
        return Err("Move pCloud Vault to Applications to start it at sign-in.".into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let contents = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>Label</key><string>dev.pcloudvault.desktop</string>
  <key>ProgramArguments</key><array><string>{}</string><string>--background</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
",
        escape(&executable.to_string_lossy())
    );
    Ok((
        PathBuf::from(home).join("Library/LaunchAgents/dev.pcloudvault.desktop.plist"),
        contents,
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn startup_entry(executable: &Path) -> AppResult<(PathBuf, String)> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("HOME is not set")?;
    // An AppImage runs from a temporary mount; start the AppImage file itself instead.
    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable.to_path_buf());
    let quoted = desktop_exec_quote(&executable.to_string_lossy());
    let contents = format!(
        "[Desktop Entry]
Type=Application
Name=pCloud Vault
Exec=\"{quoted}\" --background
X-GNOME-Autostart-enabled=true
NoDisplay=true
"
    );
    Ok((config.join("autostart/pcloud-vault.desktop"), contents))
}

/// Quotes one Exec argument for a Desktop Entry: `\`, `"`, `` ` `` and `$` get a
/// backslash inside the double quotes, `%` doubles, and the value's string escaping
/// then doubles every backslash.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn desktop_exec_quote(argument: &str) -> String {
    let mut exec_escaped = String::new();
    for c in argument.chars() {
        match c {
            '\\' | '"' | '`' | '$' => {
                exec_escaped.push('\\');
                exec_escaped.push(c);
            }
            '%' => exec_escaped.push_str("%%"),
            _ => exec_escaped.push(c),
        }
    }
    exec_escaped.replace('\\', "\\\\")
}

pub(crate) async fn task<T: Send + 'static>(
    state: &AppState,
    work: impl FnOnce(Arc<Inner>) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let inner = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || work(inner))
        .await
        .map_err(|_| {
            "The operation failed unexpectedly. Your encrypted local journal is preserved."
                .to_string()
        })?
}

#[tauri::command]
async fn status(state: State<'_, AppState>) -> AppResult<AppStatus> {
    task(&state, |s| s.status()).await
}

#[tauri::command]
async fn create_vault(
    state: State<'_, AppState>,
    password: String,
    remember: bool,
) -> AppResult<AppStatus> {
    task(&state, move |s| {
        let password = Zeroizing::new(password);
        if password.chars().count() < 12 { return Err("Choose a vault password of at least 12 characters; a long unique passphrase is recommended.".into()); }
        let mut current = locked(&s.vault)?;
        if s.vault_path().join("vault.json").exists() { return Err("A vault already exists. Unlock it instead.".into()); }
        let vault = Vault::create(&s.vault_path(), &password).map_err(|e| e.to_string())?;
        s.remember_or_warn(&vault, remember);
        *current = Some(vault);
        drop(current);
        s.log("Private vault created. Files are encrypted before leaving this computer.", "success");
        let result = s.status();
        let sync_state = s.clone();
        std::thread::spawn(move || { if let Err(e) = replication::sync(&sync_state) { sync_state.log(&e, "error"); } });
        result
    }).await
}

#[tauri::command]
async fn unlock_vault(
    state: State<'_, AppState>,
    password: String,
    remember: bool,
) -> AppResult<AppStatus> {
    task(&state, move |s| {
        let password = Zeroizing::new(password);
        let vault = Vault::open(&s.vault_path(), &password)
            .map_err(|_| "Unable to unlock. Check the password and vault integrity.".to_string())?;
        s.remember_or_warn(&vault, remember);
        *locked(&s.vault)? = Some(vault);
        s.log("Vault unlocked on this computer.", "success");
        if let Err(error) = replication::sync(&s) {
            s.log(&error, "error");
        }
        s.status()
    })
    .await
}
#[tauri::command]
async fn lock_vault(state: State<'_, AppState>) -> AppResult<AppStatus> {
    task(&state, |s| {
        *locked(&s.vault)? = None;
        s.log(
            "Vault locked. Pending encrypted writes remain safe on disk.",
            "info",
        );
        s.status()
    })
    .await
}
#[tauri::command]
async fn forget_device(state: State<'_, AppState>) -> AppResult<AppStatus> {
    task(&state, |s| {
        // A website session token stays valid until it is logged out; end it at pCloud too.
        if let Some(account) = locked(&s.account)?.clone() {
            end_web_session(&s, account);
        }
        for name in ["device.key", "account.key"] {
            let path = s.directory.join(name);
            if path.exists() {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        *locked(&s.vault)? = None;
        *locked(&s.account)? = None;
        set_startup(false)?;
        s.log(
            "Saved keys and pCloud sign-in removed from this computer.",
            "info",
        );
        s.status()
    })
    .await
}

#[tauri::command]
async fn connect_pcloud(state: State<'_, AppState>, client_id: String) -> AppResult<AppStatus> {
    task(&state, move |s| {
        let account = pcloud_api::oauth::login(client_id.trim()).map_err(|e| e.to_string())?;
        locked(&s.settings)?.client_id = client_id;
        finish_connect(&s, account)
    })
    .await
}

const WEB_LOGIN_LABEL: &str = "pcloud-login";
const WEB_LOGIN_URL: &str = "https://my.pcloud.com/#page=login";
const WEB_LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Signs in on pCloud's own website, shown in a separate window, without a registered OAuth
/// app. The window is private (no stored browsing profile) and has no access to this app's
/// commands; once pCloud sets its session cookie, Rust reads it and closes the window.
#[tauri::command]
async fn connect_pcloud_web(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AppStatus> {
    task(&state, move |s| {
        if let Some(window) = app.get_webview_window(WEB_LOGIN_LABEL) {
            let _ = window.set_focus();
            return Err("The pCloud sign-in window is already open.".into());
        }
        let url = tauri::Url::parse(WEB_LOGIN_URL).map_err(|e| e.to_string())?;
        let window = tauri::WebviewWindowBuilder::new(
            &app,
            WEB_LOGIN_LABEL,
            tauri::WebviewUrl::External(url.clone()),
        )
        .title("Sign in to pCloud")
        .inner_size(480.0, 720.0)
        .center()
        .incognito(true)
        .on_navigation(|target| target.scheme() == "https")
        // Facebook, Google and Apple sign-in open pop-ups that report back to the page.
        // They open as plain browser windows in the same private session, with no
        // access to this app's commands.
        .on_new_window(|target, _| {
            if target.scheme() == "https" {
                tauri::webview::NewWindowResponse::Allow
            } else {
                tauri::webview::NewWindowResponse::Deny
            }
        })
        .build()
        .map_err(|e| e.to_string())?;
        let closed = Arc::new(AtomicBool::new(false));
        {
            let closed = closed.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    closed.store(true, std::sync::atomic::Ordering::Release);
                }
            });
        }
        let started = std::time::Instant::now();
        let mut unreadable = 0;
        // A token that failed verification (for example one set midway through two-step
        // sign-in) is not retried; keep polling until pCloud issues a working one.
        let mut rejected: Option<Zeroizing<String>> = None;
        let client = loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if closed.load(std::sync::atomic::Ordering::Acquire)
                || app.get_webview_window(WEB_LOGIN_LABEL).is_none()
            {
                return Err("pCloud sign-in was cancelled.".into());
            }
            if started.elapsed() > WEB_LOGIN_TIMEOUT {
                let _ = window.clear_all_browsing_data();
                let _ = window.close();
                return Err("pCloud sign-in timed out. Try again.".into());
            }
            // The page can close its own web content (window.close()), leaving an empty
            // native window whose cookie store is gone; treat that as a cancel.
            let Ok(cookies) = window.cookies_for_url(url.clone()) else {
                unreadable += 1;
                if unreadable >= 6 {
                    let _ = window.clear_all_browsing_data();
                    let _ = window.close();
                    return Err("pCloud sign-in was cancelled.".into());
                }
                continue;
            };
            unreadable = 0;
            let token = cookies
                .iter()
                .find(|c| c.name() == "pcauth" && !c.value().is_empty())
                .map(|c| Zeroizing::new(c.value().to_string()));
            let location = cookies
                .iter()
                .find(|c| c.name() == "locationid")
                .and_then(|c| c.value().parse::<u64>().ok());
            drop(cookies);
            let Some(token) = token else { continue };
            if rejected.as_deref() == Some(&*token) {
                continue;
            }
            match Client::from_web_session(&token, location) {
                Ok(client) => break client,
                Err(_) => rejected = Some(token),
            }
        };
        let _ = window.clear_all_browsing_data();
        let _ = window.close();
        finish_connect(&s, client.account().clone())
    })
    .await
}

/// Logs a website session out at pCloud (OAuth tokens are left alone) and reports failures,
/// since an unended session keeps full account access.
fn end_web_session(s: &Inner, account: Account) {
    if account.auth != pcloud_api::AuthKind::WebSession {
        return;
    }
    if Client::new(account)
        .and_then(|client| client.logout())
        .is_err()
    {
        s.log(
            "Could not end a pCloud website session. You can end it in pCloud's account settings.",
            "error",
        );
    }
}

/// Saves a verified account with device protection and connects the remote vault.
fn finish_connect(s: &Arc<Inner>, account: pcloud_api::Account) -> AppResult<AppStatus> {
    let previous = locked(&s.account)?.clone();
    if let Some(previous) = &previous {
        if previous.user_id != account.user_id {
            end_web_session(s, account);
            return Err("Forget this computer before switching pCloud accounts. The local vault will be preserved.".into());
        }
    }
    let bytes = Zeroizing::new(serde_json::to_vec(&account).map_err(|e| e.to_string())?);
    let saved = s.directory.join("account.key");
    match pcloud_api::device_protection::protect(&bytes) {
        Ok(protected) => atomic_write(&saved, &protected)?,
        // Without protected storage, stay connected for this session only.
        Err(error) => {
            if saved.exists() {
                fs::remove_file(&saved).map_err(|e| e.to_string())?;
            }
            s.log(
                &format!("{error} pCloud stays connected until the app quits."),
                "error",
            );
        }
    }
    let replaced = previous.filter(|previous| previous.token != account.token);
    *locked(&s.account)? = Some(account);
    if let Some(replaced) = replaced {
        end_web_session(s, replaced);
    }
    replication::discover(s)?;
    s.save_settings()?;
    if locked(&s.vault)?.is_some() {
        replication::sync(s)?;
    }
    s.log("pCloud connected securely.", "success");
    s.status()
}

#[tauri::command]
async fn save_settings(
    state: State<'_, AppState>,
    cache_limit_gib: u64,
    client_id: String,
) -> AppResult<AppStatus> {
    task(&state, move |s| {
        if !(1..=4096).contains(&cache_limit_gib) {
            return Err("Cache size must be between 1 and 4096 GiB.".into());
        }
        {
            let mut settings = locked(&s.settings)?;
            settings.cache_limit_gib = cache_limit_gib;
            settings.client_id = client_id;
        }
        s.save_settings()?;
        s.status()
    })
    .await
}

pub(crate) fn cloud_id(value: &str) -> AppResult<(u64, bool)> {
    let (kind, number) = value.split_at_checked(1).ok_or("Invalid cloud entry")?;
    if !matches!(kind, "f" | "d") {
        return Err("Invalid cloud entry type".into());
    }
    Ok((
        number.parse().map_err(|_| "Invalid cloud entry ID")?,
        kind == "d",
    ))
}
pub(crate) fn cloud_safe(s: &Inner, client: &Client, id: u64, folder: bool) -> AppResult<()> {
    let reserved = locked(&s.settings)?.remote_folder;
    let mut current = Some((id, folder));
    for _ in 0..256 {
        let Some((id, folder)) = current else {
            return Ok(());
        };
        if folder && (reserved == Some(id)) {
            return Err("The encrypted vault is managed only through Private Vault.".into());
        }
        if id == 0 && folder {
            return Ok(());
        }
        let entry = client.stat(id, folder).map_err(|e| e.to_string())?;
        if entry.is_folder && entry.name.eq_ignore_ascii_case(".pcloud-vault") {
            return Err("The vault storage folder is protected.".into());
        }
        current = Some((entry.parent_folder_id, true));
    }
    Err("Cloud directory ancestry exceeds the supported depth.".into())
}

#[tauri::command]
async fn list_entries(
    state: State<'_, AppState>,
    location: String,
    parent_id: Option<String>,
    query: Option<String>,
    view: String,
) -> AppResult<Vec<Entry>> {
    task(&state, move |s| {
        let mut entries = if location == "cloud" {
            let client = s.client()?;
            let folder = parent_id
                .as_deref()
                .map(cloud_id)
                .transpose()?
                .map(|(id, folder)| {
                    if folder {
                        Ok(id)
                    } else {
                        Err("Select a folder")
                    }
                })
                .transpose()?
                .unwrap_or(0);
            cloud_safe(&s, &client, folder, true)?;
            client
                .list_folder(folder)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|m| {
                    !m.name.eq_ignore_ascii_case(".pcloud-vault")
                        && Some(m.id) != locked(&s.settings).ok().and_then(|set| set.remote_folder)
                })
                .map(|m| Entry {
                    id: format!("{}{}", if m.is_folder { "d" } else { "f" }, m.id),
                    name: m.name,
                    parent_id: if folder == 0 {
                        None
                    } else {
                        Some(format!("d{folder}"))
                    },
                    kind: if m.is_folder {
                        EntryKind::Folder
                    } else {
                        EntryKind::File
                    },
                    size: m.size,
                    modified: m.modified * 1000,
                    starred: false,
                    pinned: false,
                    trashed: false,
                    sync_state: "synced".into(),
                })
                .collect()
        } else if location == "vault" {
            let vault = locked(&s.vault)?;
            let vault = vault.as_ref().ok_or("Unlock your vault first")?;
            if view == "files" && query.as_deref().unwrap_or("").is_empty() {
                vault.list(parent_id.as_deref(), false)
            } else {
                vault.all_entries()
            }
        } else {
            return Err("Unknown storage location".into());
        };
        let query = query.unwrap_or_default().to_lowercase();
        entries.retain(|entry| {
            let visible = match view.as_str() {
                "favorites" => entry.starred && !entry.trashed,
                "offline" => entry.pinned && !entry.trashed,
                "trash" => entry.trashed,
                _ => !entry.trashed,
            };
            visible && (query.is_empty() || entry.name.to_lowercase().contains(&query))
        });
        entries.sort_by(|a, b| {
            (a.kind == EntryKind::File)
                .cmp(&(b.kind == EntryKind::File))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(entries)
    })
    .await
}

#[tauri::command]
async fn new_folder(
    state: State<'_, AppState>,
    location: String,
    parent_id: Option<String>,
    name: String,
) -> AppResult<()> {
    task(&state, move |s| {
        if location == "cloud" {
            let client = s.client()?;
            let parent = parent_id
                .as_deref()
                .map(cloud_id)
                .transpose()?
                .map(|(id, _)| id)
                .unwrap_or(0);
            cloud_safe(&s, &client, parent, true)?;
            if name.eq_ignore_ascii_case(".pcloud-vault") {
                return Err("That folder name is reserved for encrypted storage.".into());
            }
            client
                .create_folder(parent, &name)
                .map_err(|e| e.to_string())?;
        } else if location == "vault" {
            locked(&s.vault)?
                .as_mut()
                .ok_or("Unlock your vault first")?
                .create_folder(parent_id.as_deref(), &name)
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Unknown storage location".into());
        }
        s.log("Folder created.", "success");
        Ok(())
    })
    .await
}

#[tauri::command]
async fn rename_entry(
    state: State<'_, AppState>,
    location: String,
    id: String,
    name: String,
) -> AppResult<()> {
    task(&state, move |s| {
        if location == "cloud" {
            let client = s.client()?;
            let (id, folder) = cloud_id(&id)?;
            cloud_safe(&s, &client, id, folder)?;
            if name.eq_ignore_ascii_case(".pcloud-vault") {
                return Err("That folder name is reserved.".into());
            }
            if folder {
                client.rename_folder(id, &name)
            } else {
                client.rename_file(id, &name)
            }
            .map_err(|e| e.to_string())?;
        } else if location == "vault" {
            locked(&s.vault)?
                .as_mut()
                .ok_or("Unlock your vault first")?
                .rename(&id, &name)
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Unknown storage location".into());
        }
        s.log("Entry renamed.", "success");
        Ok(())
    })
    .await
}

#[tauri::command]
async fn move_entry(
    state: State<'_, AppState>,
    location: String,
    id: String,
    parent_id: Option<String>,
) -> AppResult<()> {
    task(&state, move |s| {
        if location == "cloud" {
            let client = s.client()?;
            let (id, folder) = cloud_id(&id)?;
            cloud_safe(&s, &client, id, folder)?;
            let parent = parent_id
                .as_deref()
                .map(cloud_id)
                .transpose()?
                .map(|(id, _)| id)
                .unwrap_or(0);
            cloud_safe(&s, &client, parent, true)?;
            if folder {
                client.move_folder(id, parent)
            } else {
                client.move_file(id, parent)
            }
            .map_err(|e| e.to_string())?;
        } else if location == "vault" {
            locked(&s.vault)?
                .as_mut()
                .ok_or("Unlock your vault first")?
                .move_entry(&id, parent_id.as_deref())
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Unknown storage location".into());
        }
        s.log("Entry moved.", "success");
        Ok(())
    })
    .await
}

#[tauri::command]
async fn trash_entry(state: State<'_, AppState>, location: String, id: String) -> AppResult<()> {
    task(&state, move |s| {
        if location == "cloud" {
            let client = s.client()?;
            let (id, folder) = cloud_id(&id)?;
            cloud_safe(&s, &client, id, folder)?;
            if folder {
                client.delete_folder(id)
            } else {
                client.delete_file(id)
            }
            .map_err(|e| e.to_string())?;
        } else if location == "vault" {
            locked(&s.vault)?
                .as_mut()
                .ok_or("Unlock your vault first")?
                .set_trashed(&id, true)
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Unknown storage location".into());
        }
        s.log("Entry moved to trash.", "success");
        Ok(())
    })
    .await
}
#[tauri::command]
async fn restore_entry(state: State<'_, AppState>, location: String, id: String) -> AppResult<()> {
    task(&state, move |s| {
        if location != "vault" {
            return Err("Restore ordinary files using pCloud's Trash.".into());
        }
        locked(&s.vault)?
            .as_mut()
            .ok_or("Unlock your vault first")?
            .set_trashed(&id, false)
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
#[tauri::command]
async fn toggle_favorite(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        let mut vault = locked(&s.vault)?;
        let vault = vault.as_mut().ok_or("Unlock your vault first")?;
        let entry = vault.entry(&id).map_err(|e| e.to_string())?;
        vault
            .set_favorite(&id, !entry.starred)
            .map_err(|e| e.to_string())
    })
    .await
}
#[tauri::command]
async fn toggle_pin(state: State<'_, AppState>, id: String) -> AppResult<()> {
    task(&state, move |s| {
        let pinned = locked(&s.vault)?
            .as_ref()
            .ok_or("Unlock your vault first")?
            .entry(&id)
            .map_err(|e| e.to_string())?
            .pinned;
        // Keep the hydrated chunks protected until the pin is recorded.
        let _in_use = if pinned {
            None
        } else {
            Some(replication::ensure_file(&s, &id)?)
        };
        let mut vault = locked(&s.vault)?;
        let vault = vault.as_mut().ok_or("Unlock your vault first")?;
        let entry = vault.entry(&id).map_err(|e| e.to_string())?;
        vault
            .set_pinned(&id, !entry.pinned)
            .map_err(|e| e.to_string())
    })
    .await
}

pub(crate) fn import_local(
    s: &Arc<Inner>,
    paths: Vec<PathBuf>,
    parent: Option<String>,
) -> AppResult<()> {
    let paths = paths
        .into_iter()
        .map(|path| {
            let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
            outside_storage(s, &path)?;
            let storage = fs::canonicalize(&s.directory).map_err(|e| e.to_string())?;
            if path_key(&storage).starts_with(path_key(&path)) { return Err("This folder contains the client's encrypted storage. Select a narrower source folder to prevent importing the vault into itself.".into()); }
            Ok(path)
        })
        .collect::<AppResult<Vec<_>>>()?;
    let mut vault = locked(&s.vault)?;
    let vault = vault.as_mut().ok_or("Unlock your vault first")?;
    fn walk(
        transfers: &crate::transfers::Transfers,
        vault: &mut Vault,
        path: &Path,
        parent: Option<&str>,
        depth: usize,
    ) -> AppResult<()> {
        if depth > 128 {
            return Err("Folder import exceeds the supported depth.".into());
        }
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(
                "Symbolic links are not imported. Select the original file or folder.".into(),
            );
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid filename")?;
        if metadata.is_dir() {
            let folder = vault
                .create_folder(parent, name)
                .map_err(|e| e.to_string())?;
            for child in fs::read_dir(path).map_err(|e| e.to_string())? {
                walk(
                    transfers,
                    vault,
                    &child.map_err(|e| e.to_string())?.path(),
                    Some(&folder.id),
                    depth + 1,
                )?;
            }
        } else if metadata.is_file() {
            transfers.track(
                name,
                crate::transfers::Direction::Upload,
                true,
                metadata.len(),
                || {
                    vault
                        .import_file(parent, name, File::open(path).map_err(|e| e.to_string())?)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                },
            )?;
        } else {
            return Err("Only regular files and folders can be imported.".into());
        }
        Ok(())
    }
    for path in paths {
        walk(&s.transfers, vault, &path, parent.as_deref(), 0)?;
    }
    s.log(
        "Import saved locally with encryption. Originals were preserved.",
        "success",
    );
    Ok(())
}

/// Canonical path for containment checks. Windows and macOS file systems are
/// case-insensitive by default, so compare those case-insensitively.
pub(crate) fn path_key(path: &Path) -> PathBuf {
    if cfg!(any(windows, target_os = "macos")) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

pub(crate) fn outside_storage(s: &Inner, path: &Path) -> AppResult<()> {
    let candidate = if path.exists() {
        fs::canonicalize(path).map_err(|e| e.to_string())?
    } else {
        fs::canonicalize(path.parent().ok_or("Choose an absolute output path")?)
            .map_err(|e| e.to_string())?
            .join(path.file_name().ok_or("Invalid output filename")?)
    };
    let storage = fs::canonicalize(&s.directory).map_err(|e| e.to_string())?;
    if path_key(&candidate).starts_with(path_key(&storage)) {
        return Err(
            "Choose a location outside this client's encrypted application storage.".into(),
        );
    }
    Ok(())
}

fn choose_vault_export(s: &Arc<Inner>, id: &str) -> AppResult<Option<PathBuf>> {
    let _in_use = replication::ensure_file(s, id)?;
    let entry = locked(&s.vault)?
        .as_ref()
        .ok_or("Unlock your vault first")?
        .entry(id)
        .map_err(|e| e.to_string())?;
    let path = if entry.kind == EntryKind::Folder {
        rfd::FileDialog::new()
            .set_title("Choose where to export a decrypted folder copy")
            .pick_folder()
            .map(|parent| parent.join(&entry.name))
    } else {
        rfd::FileDialog::new()
            .set_file_name(&entry.name)
            .set_title("Export decrypted copy — save outside the vault")
            .save_file()
    };
    let Some(path) = path else {
        return Ok(None);
    };
    outside_storage(s, &path)?;
    if path.exists() {
        return Err("That destination already exists. Choose a new filename or folder to preserve existing files.".into());
    }
    export_vault_to(s, id, &path)?;
    Ok(Some(path))
}

pub(crate) fn export_vault_to(s: &Inner, id: &str, destination: &Path) -> AppResult<()> {
    let vault = locked(&s.vault)?;
    let vault = vault.as_ref().ok_or("Unlock your vault first")?;
    let entry = vault.entry(id).map_err(|e| e.to_string())?;
    if entry.kind == EntryKind::File {
        return vault
            .export_file(id, destination)
            .map_err(|e| e.to_string());
    }
    fs::create_dir(destination).map_err(|e| e.to_string())?;
    fn walk(vault: &Vault, id: &str, path: &Path, depth: usize) -> AppResult<()> {
        if depth > 128 {
            return Err("Folder export exceeds the supported depth".into());
        }
        for child in vault.list(Some(id), false) {
            let destination = path.join(&child.name);
            if child.kind == EntryKind::Folder {
                fs::create_dir(&destination).map_err(|e| e.to_string())?;
                walk(vault, &child.id, &destination, depth + 1)?;
            } else {
                vault
                    .export_file(&child.id, &destination)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    walk(vault, id, destination, 0)
}
#[tauri::command]
async fn import_files(state: State<'_, AppState>, parent_id: Option<String>) -> AppResult<()> {
    task(&state, move |s| {
        let Some(paths) = rfd::FileDialog::new()
            .set_title("Import files into Private Vault")
            .pick_files()
        else {
            return Ok(());
        };
        import_local(&s, paths, parent_id)
    })
    .await
}
#[tauri::command]
async fn import_paths(
    state: State<'_, AppState>,
    paths: Vec<String>,
    parent_id: Option<String>,
) -> AppResult<()> {
    task(&state, move |s| {
        if paths.len() > 1024 {
            return Err("Import at most 1024 top-level items at a time.".into());
        }
        import_local(
            &s,
            paths.into_iter().map(PathBuf::from).collect(),
            parent_id,
        )
    })
    .await
}
#[tauri::command]
async fn import_cloud_file(
    state: State<'_, AppState>,
    id: String,
    parent_id: Option<String>,
) -> AppResult<()> {
    task(&state, move |s| {
        let client = s.client()?; let (id, folder) = cloud_id(&id)?; cloud_safe(&s, &client, id, folder)?;
        if folder { return Err("Import individual pCloud files, or download the folder and drag it into Private Vault.".into()); }
        import_cloud_into_vault(&s, client, id, parent_id.as_deref())?;
        replication::sync(&s)?;
        s.log("Imported and verified an encrypted cloud copy. Original preserved.", "success"); Ok(())
    }).await
}

/// Streams one pCloud file into the vault and verifies its size before keeping it.
pub(crate) fn import_cloud_into_vault(
    s: &Arc<Inner>,
    client: Client,
    id: u64,
    parent_id: Option<&str>,
) -> AppResult<Entry> {
    let metadata = client.stat(id, false).map_err(|e| e.to_string())?;
    let tracker = s.transfers.start(
        &metadata.name,
        crate::transfers::Direction::Upload,
        true,
        metadata.size,
    );
    let (reader, mut writer) = os_pipe::pipe().map_err(|e| e.to_string())?;
    let download = std::thread::spawn(move || client.download_to(id, &mut writer));
    let result = {
        let mut vault = locked(&s.vault)?;
        let vault = vault.as_mut().ok_or("Unlock your vault first")?;
        let imported = vault
            .import_file(parent_id, &metadata.name, reader)
            .map_err(|e| e.to_string());
        let transferred = download
            .join()
            .map_err(|_| "Cloud import interrupted".to_string())?
            .map_err(|e| e.to_string());
        match (imported, transferred) {
            (Ok(entry), Ok(size)) if size == metadata.size && entry.size == size => Ok(entry),
            (Ok(entry), _) => {
                vault
                    .set_trashed(&entry.id, true)
                    .map_err(|e| e.to_string())?;
                Err("Import was incomplete; the original remains untouched.".to_string())
            }
            (Err(error), _) => Err(error),
        }
    };
    s.transfers
        .finish(&tracker, &result.as_ref().map(|_| ()).map_err(Clone::clone));
    result
}

#[tauri::command]
async fn download_entry(state: State<'_, AppState>, location: String, id: String) -> AppResult<()> {
    task(&state, move |s| {
        if location == "vault" {
            if choose_vault_export(&s, &id)?.is_none() {
                return Ok(());
            }
        } else if location == "cloud" {
            let client = s.client()?;
            let (id, folder) = cloud_id(&id)?;
            cloud_safe(&s, &client, id, folder)?;
            if folder {
                return Err("Download individual files from pCloud Files.".into());
            }
            let metadata = client.stat(id, false).map_err(|e| e.to_string())?;
            let Some(path) = rfd::FileDialog::new()
                .set_file_name(&metadata.name)
                .save_file()
            else {
                return Ok(());
            };
            outside_storage(&s, &path)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .map_err(|e| e.to_string())?;
            match client.download_to(id, &mut file) {
                Ok(n) if n == metadata.size => file.sync_all().map_err(|e| e.to_string())?,
                _ => {
                    drop(file);
                    let _ = fs::remove_file(path);
                    return Err("Download incomplete. Please retry.".into());
                }
            }
        } else {
            return Err("Unknown storage location".into());
        }
        s.log("File exported to the selected location.", "success");
        Ok(())
    })
    .await
}

#[tauri::command]
async fn open_entry(state: State<'_, AppState>, location: String, id: String) -> AppResult<()> {
    task(&state, move |s| {
        if location != "vault" {
            return Err("Download ordinary pCloud files to open them locally.".into());
        }
        let Some(path) = choose_vault_export(&s, &id)? else {
            return Ok(());
        };
        open::that(path).map_err(|_| {
            "Your system could not open this file with its default application.".to_string()
        })
    })
    .await
}

#[tauri::command]
async fn preview_entry(
    state: State<'_, AppState>,
    location: String,
    id: String,
) -> AppResult<Option<String>> {
    task(&state, move |s| preview_data(&s, &location, &id)).await
}

/// Image bytes as a data URL for previews and thumbnails (PNG, JPEG, GIF, WebP up to 20 MiB).
pub(crate) fn preview_data(s: &Arc<Inner>, location: &str, id: &str) -> AppResult<Option<String>> {
    const MAX: u64 = 20 * 1024 * 1024;
    let mut bytes = Zeroizing::new(Vec::new());
    if location == "vault" {
        let _in_use = replication::ensure_file(s, id)?;
        let vault = locked(&s.vault)?;
        let vault = vault.as_ref().ok_or("Unlock your vault first")?;
        let entry = vault.entry(id).map_err(|e| e.to_string())?;
        if entry.kind == EntryKind::Folder || entry.size > MAX {
            return Ok(None);
        }
        bytes.resize(entry.size as usize, 0);
        let mut offset = 0;
        while offset < bytes.len() {
            let count = vault
                .read(id, offset as u64, &mut bytes[offset..])
                .map_err(|e| e.to_string())?;
            if count == 0 {
                return Err("Preview is incomplete".into());
            }
            offset += count;
        }
    } else if location == "cloud" {
        let client = s.client()?;
        let (id, folder) = cloud_id(id)?;
        cloud_safe(s, &client, id, folder)?;
        if folder || client.stat(id, false).map_err(|e| e.to_string())?.size > MAX {
            return Ok(None);
        }
        let mut writer = replication::BoundedWriter::new(MAX as usize);
        client
            .download_to(id, &mut writer)
            .map_err(|e| e.to_string())?;
        *bytes = writer.into_inner();
    } else {
        return Err("Unknown storage location".into());
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Ok(None);
    };
    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes.as_slice())
    )))
}
#[tauri::command]
async fn sync_now(state: State<'_, AppState>) -> AppResult<()> {
    task(&state, |s| replication::sync(&s)).await
}
#[tauri::command]
async fn change_password(
    state: State<'_, AppState>,
    old_password: String,
    new_password: String,
) -> AppResult<()> {
    task(&state, move |s| {
        let old = Zeroizing::new(old_password); let new = Zeroizing::new(new_password);
        if new.chars().count() < 12 { return Err("Use a password of at least 12 characters.".into()); }
        let _guard = replication::SyncGuard::acquire(&s)?;
        {
            let mut vault = locked(&s.vault)?; let vault = vault.as_mut().ok_or("Unlock your vault first")?;
            let prepared = vault.prepare_password_change(&old, &new).map_err(|e| e.to_string())?;
            atomic_write(&s.directory.join("pending-header.json"), &serde_json::to_vec(&prepared).map_err(|e| e.to_string())?)?;
            let superseded = vault.header().salt.clone();
            vault.adopt_header(&prepared).map_err(|e| e.to_string())?;
            replication::retire_header(&s, &superseded)?;
        }
        if locked(&s.account)?.is_none() {
            // Nothing to publish: the adopted local header is authoritative. A stale pending
            // wrapper must not outlive this vault and be replayed over a different one later.
            fs::remove_file(s.directory.join("pending-header.json")).map_err(|e| e.to_string())?;
        } else {
            if let Err(error) = replication::publish_header(&s) { s.log("Password changed locally; the encrypted cloud update is pending and will retry automatically.", "info"); return Err(format!("Password changed on this computer. Cloud update is pending: {error}")); }
        }
        s.log("Vault password changed. Already trusted computers retain their keys.", "success"); Ok(())
    }).await
}
#[tauri::command]
async fn export_recovery(state: State<'_, AppState>) -> AppResult<()> {
    task(&state, |s| {
        if locked(&s.vault)?.is_none() {
            return Err("Unlock your vault first".into());
        }
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("vault.pcloud-recovery")
            .set_title("Save recovery key somewhere safe and separate")
            .save_file()
        else {
            return Ok(());
        };
        outside_storage(&s, &path)?;
        let guard = locked(&s.vault)?;
        let vault = guard
            .as_ref()
            .ok_or("Vault was locked before recovery export. Unlock it and try again.")?;
        let key = Zeroizing::new(vault.recovery_key());
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(key.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        s.log(
            "Recovery key exported. Keep it private and separate from the vault.",
            "success",
        );
        Ok(())
    })
    .await
}
#[tauri::command]
async fn get_activity(state: State<'_, AppState>) -> AppResult<Vec<Activity>> {
    task(&state, |s| {
        Ok(locked(&s.activity)?.iter().cloned().collect())
    })
    .await
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let args: Vec<_> = std::env::args_os().collect();
            let directory = if let Some(index) = args.iter().position(|arg| arg == "--data-dir") {
                PathBuf::from(
                    args.get(index + 1)
                        .ok_or_else(|| std::io::Error::other("--data-dir requires a directory"))?,
                )
            } else {
                app.path().app_data_dir()?
            };
            fs::create_dir_all(&directory)?;
            // Pending wrappers contain no plaintext secret. Replay a prepared
            // password update before any password/DPAPI unlock after a crash.
            if let Ok(pending) = fs::read(directory.join("pending-header.json")) {
                let header = serde_json::from_slice::<vault_core::VaultHeader>(&pending).ok();
                let current = vault_core::Vault::read_header(&directory.join("vault")).ok();
                match (header, current) {
                    (Some(header), Some(current))
                        if header.validate().is_ok() && header.vault_id == current.vault_id =>
                    {
                        atomic_write(&directory.join("vault/vault.json"), &pending)
                            .map_err(std::io::Error::other)?;
                    }
                    // A wrapper for another (or no) vault must never replace this vault's
                    // only key envelope; discard it.
                    _ => fs::remove_file(directory.join("pending-header.json"))?,
                }
            }
            let settings = fs::read(directory.join("settings.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default();
            let account = fs::read(directory.join("account.key"))
                .ok()
                .and_then(|bytes| pcloud_api::device_protection::unprotect(&bytes).ok())
                .map(Zeroizing::new)
                .and_then(|bytes| serde_json::from_slice(&bytes).ok());
            let inner = Arc::new(Inner {
                directory,
                vault: Arc::new(Mutex::new(None)),
                account: Mutex::new(account),
                settings: Mutex::new(settings),
                activity: Mutex::new(VecDeque::new()),
                syncing: AtomicBool::new(false),
                in_use: Mutex::default(),
                transfers: Default::default(),
                jobs: Default::default(),
            });
            if let Ok(blob) = fs::read(inner.directory.join("device.key")) {
                if let Ok(bytes) = pcloud_api::device_protection::unprotect(&blob) {
                    let bytes = Zeroizing::new(bytes);
                    if let Ok((id, key)) = serde_json::from_slice::<(String, [u8; 32])>(&bytes) {
                        let key = Zeroizing::new(key);
                        if let Ok(vault) = Vault::from_key(&inner.vault_path(), *key) {
                            if vault.vault_id() == id {
                                *locked(&inner.vault).map_err(std::io::Error::other)? = Some(vault);
                            }
                        }
                    }
                }
            }
            app.manage(AppState(inner.clone()));
            let show = MenuItem::with_id(app, "show", "Open pCloud Vault", true, None::<&str>)?;
            let lock = MenuItem::with_id(app, "lock", "Lock vault", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &lock, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("pCloud Vault — encrypted cloud files");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "lock" => {
                    let inner = app.state::<AppState>().0.clone();
                    std::thread::spawn(move || {
                        if let Ok(mut vault) = locked(&inner.vault) {
                            *vault = None;
                        }
                        inner.log("Vault locked.", "info");
                    });
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            })
            .build(app)?;
            if args.iter().any(|arg| arg == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
            }
            let startup = inner.clone();
            std::thread::spawn(move || {
                let mut round: u64 = 0;
                loop {
                    let connected = locked(&startup.account).is_ok_and(|a| a.is_some());
                    if connected && locked(&startup.vault).is_ok_and(|v| v.is_some()) {
                        if let Err(error) = replication::sync(&startup) {
                            startup.log(&error, "error");
                        }
                        // About every two minutes: encrypt files received by file requests.
                        if round.is_multiple_of(8) {
                            if let Err(error) = crate::features::encrypt_received(&startup) {
                                startup.log(&error, "error");
                            }
                        }
                    }
                    round += 1;
                    std::thread::sleep(std::time::Duration::from_secs(15));
                }
            });
            let scheduler = inner.clone();
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(10));
                crate::jobs::tick(&scheduler, &handle);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            status,
            list_entries,
            create_vault,
            unlock_vault,
            lock_vault,
            forget_device,
            connect_pcloud,
            connect_pcloud_web,
            save_settings,
            new_folder,
            rename_entry,
            move_entry,
            trash_entry,
            restore_entry,
            toggle_favorite,
            toggle_pin,
            import_files,
            import_paths,
            import_cloud_file,
            download_entry,
            open_entry,
            preview_entry,
            sync_now,
            change_password,
            export_recovery,
            get_activity,
            crate::features::get_preferences,
            crate::features::set_preferences,
            crate::features::account_info,
            crate::features::known_folders,
            crate::features::pick_folder,
            crate::features::open_url,
            crate::features::upload_paths,
            crate::features::upload_files,
            crate::features::upload_folder,
            crate::features::get_transfers,
            crate::features::clear_transfers,
            crate::features::list_photos,
            crate::features::photo_thumb,
            crate::features::list_links,
            crate::features::create_link,
            crate::features::delete_link,
            crate::features::list_shares,
            crate::features::share_folder,
            crate::features::accept_share,
            crate::features::decline_share,
            crate::features::remove_share,
            crate::features::list_file_requests,
            crate::features::create_file_request,
            crate::features::delete_file_request,
            crate::jobs::list_jobs,
            crate::jobs::add_job,
            crate::jobs::update_job,
            crate::jobs::remove_job,
            crate::jobs::run_job,
            crate::jobs::resolve_guard,
            crate::jobs::open_recycle
        ])
        .build(tauri::generate_context!())
        .expect("Unable to start pCloud Vault")
        .run(|_app, _event| {
            // macOS: clicking the Dock icon shows the window that closing hid.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                if let Some(window) = _app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_entry_exec_arguments_are_quoted() {
        assert_eq!(
            desktop_exec_quote("/opt/pCloud Vault/app"),
            "/opt/pCloud Vault/app"
        );
        // Exec quoting adds one backslash; Desktop Entry string escaping doubles it.
        assert_eq!(desktop_exec_quote(r#"/a"b"#), r#"/a\\"b"#);
        assert_eq!(desktop_exec_quote("/a$b`c"), r"/a\\$b\\`c");
        assert_eq!(desktop_exec_quote(r"/a\b"), r"/a\\\\b");
        assert_eq!(desktop_exec_quote("/100%"), "/100%%");
    }
    #[test]
    fn storage_containment_compares_whole_path_components() {
        let storage = Path::new("/data/vault");
        assert!(path_key(Path::new("/data/vault/objects")).starts_with(path_key(storage)));
        assert!(!path_key(Path::new("/data/vault-export")).starts_with(path_key(storage)));
    }
    fn fixture() -> (tempfile::TempDir, Arc<Inner>) {
        let directory = tempfile::tempdir().unwrap();
        let inner = Arc::new(Inner {
            directory: directory.path().to_path_buf(),
            vault: Arc::new(Mutex::new(None)),
            account: Mutex::new(None),
            settings: Mutex::new(Settings::default()),
            activity: Mutex::new(VecDeque::new()),
            syncing: AtomicBool::new(false),
            in_use: Mutex::default(),
            transfers: Default::default(),
            jobs: Default::default(),
        });
        *inner.vault.lock().unwrap() =
            Some(Vault::create(&inner.vault_path(), "test-only-long-passphrase").unwrap());
        (directory, inner)
    }
    #[test]
    fn standalone_folder_import_export_and_pin_preserve_originals() {
        let (_storage, inner) = fixture();
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("Papers")).unwrap();
        fs::write(
            source.path().join("Papers/note.txt"),
            b"private-marker-8de7a2",
        )
        .unwrap();
        import_local(&inner, vec![source.path().join("Papers")], None).unwrap();
        let folder = inner
            .vault
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .list(None, false)
            .pop()
            .unwrap();
        replication::ensure_file(&inner, &folder.id).unwrap();
        inner
            .vault
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .set_pinned(&folder.id, true)
            .unwrap();
        let destination = source.path().join("Export");
        export_vault_to(&inner, &folder.id, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("note.txt")).unwrap(),
            b"private-marker-8de7a2"
        );
        assert_eq!(
            fs::read(source.path().join("Papers/note.txt")).unwrap(),
            b"private-marker-8de7a2"
        );
        assert!(export_vault_to(&inner, &folder.id, &destination).is_err());
        let status = inner.status().unwrap();
        assert!(status.unlocked && status.vault_exists);
        assert!(!status.connected);
        assert!(status.pending_count > 0);
        assert!(outside_storage(&inner, &inner.directory.join("plaintext.txt")).is_err());
    }
    #[test]
    fn cloud_identifiers_and_plaintext_export_boundaries_are_checked() {
        assert_eq!(cloud_id("d123").unwrap(), (123, true));
        assert!(cloud_id("123").is_err());
        assert!(cloud_id("😀123").is_err());
        assert!(cloud_id("f../").is_err());
        let (_storage, inner) = fixture();
        assert!(outside_storage(&inner, &inner.vault_path().join("vault.json")).is_err());
        assert!(import_local(&inner, vec![inner.vault_path()], None).is_err());
        assert!(import_local(
            &inner,
            vec![inner.directory.parent().unwrap().to_path_buf()],
            None
        )
        .is_err());
        let temporary = tempfile::tempdir().unwrap();
        atomic_write(&temporary.path().join("settings"), b"first").unwrap();
        atomic_write(&temporary.path().join("settings"), b"second").unwrap();
        assert_eq!(
            fs::read(temporary.path().join("settings")).unwrap(),
            b"second"
        );
    }
}
