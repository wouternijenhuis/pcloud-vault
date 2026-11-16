//! pCloud's regional API, browser sign-in, and Windows user-bound secret storage.
//! Network errors intentionally omit request URLs and response bodies.

pub mod device_protection;
pub mod oauth;

use reqwest::blocking::{multipart, Client as HttpClient, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, Cursor, Read, Write},
    time::Duration,
};
use thiserror::Error;
use url::Url;
use zeroize::Zeroize;

pub type Result<T> = std::result::Result<T, Error>;
const MAX_API_RESPONSE: u64 = 64 * 1024 * 1024;
const MAX_THUMBNAIL: u64 = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Unable to reach pCloud. Check your connection and try again.")]
    Network,
    #[error("pCloud returned HTTP status {0}.")]
    Http(u16),
    #[error("pCloud sign-in expired or was revoked. Connect your account again.")]
    Authentication,
    #[error("pCloud denied access. Ensure this registered API app has access to your existing account files, not only its application folder.")]
    AccessDenied,
    #[error("pCloud storage quota is full.")]
    Quota,
    #[error("The pCloud file or folder no longer exists.")]
    NotFound,
    #[error("pCloud rejected this operation (code {0}).")]
    Api(u64),
    #[error("pCloud returned an invalid or oversized response.")]
    InvalidResponse,
    #[error("An unsafe download address was rejected.")]
    UnsafeDownload,
    #[error("The uploaded encrypted object did not match its expected bytes.")]
    Integrity,
    #[error("Invalid filename.")]
    InvalidName,
    #[error("Invalid email address, comment, or option.")]
    InvalidInput,
    #[error("The local file could not be read or written.")]
    Io,
    #[error("A registered pCloud OAuth client ID is required.")]
    MissingClientId,
    #[error("Could not start local sign-in callback on 127.0.0.1:43187. Close another sign-in attempt and try again.")]
    CallbackUnavailable,
    #[error("Could not open the system browser for pCloud sign-in.")]
    Browser,
    #[error("pCloud sign-in timed out. Try connecting again.")]
    LoginTimeout,
    #[error("pCloud sign-in was declined.")]
    LoginDeclined,
    #[error("The pCloud sign-in response was invalid.")]
    InvalidLogin,
    #[error("This computer could not protect or unlock its saved secret. Use the vault password and connect pCloud again if needed.")]
    DeviceProtection,
    #[error("The system keyring is unavailable or access was denied, so this computer cannot save secrets. Allow access in the Keychain prompt on macOS, or run a Secret Service such as GNOME Keyring or KWallet on Linux.")]
    KeyringUnavailable,
}

impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Us,
    Eu,
}

impl Region {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Us => "us",
            Self::Eu => "eu",
        }
    }
    pub fn api_host(self) -> &'static str {
        match self {
            Self::Us => "api.pcloud.com",
            Self::Eu => "eapi.pcloud.com",
        }
    }
    /// Maps pCloud's web `locationid` (1 = US, 2 = EU) to the matching API region.
    pub fn from_location_id(location_id: u64) -> Result<Self> {
        match location_id {
            1 => Ok(Self::Us),
            2 => Ok(Self::Eu),
            _ => Err(Error::InvalidLogin),
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::Us => Self::Eu,
            Self::Eu => Self::Us,
        }
    }
    pub fn from_oauth(location_id: u64, host: &str) -> Result<Self> {
        match (location_id, host) {
            (1, "api.pcloud.com") => Ok(Self::Us),
            (2, "eapi.pcloud.com") => Ok(Self::Eu),
            _ => Err(Error::InvalidLogin),
        }
    }
}

/// How the account token was obtained, which decides the API parameter that carries it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthKind {
    /// OAuth 2.0 bearer token from a registered pCloud app (`access_token`).
    #[default]
    OAuth,
    /// Session token from signing in on pCloud's own website (`auth`).
    WebSession,
}

/// Serialize only inside the DPAPI protected account record, never plain JSON files.
#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub token: String,
    pub region: Region,
    pub user_id: u64,
    pub email: String,
    #[serde(default)]
    pub auth: AuthKind,
}

impl Account {
    fn token_parameter(&self) -> &'static str {
        match self.auth {
            AuthKind::OAuth => "access_token",
            AuthKind::WebSession => "auth",
        }
    }
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("token", &"[redacted]")
            .field("region", &self.region)
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub id: u64,
    pub name: String,
    pub is_folder: bool,
    pub parent_folder_id: u64,
    pub size: u64,
    pub modified: u64,
    /// pCloud content hash; changes whenever the file content changes. Absent for folders.
    #[serde(default)]
    pub hash: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfo {
    pub user_id: u64,
    pub email: String,
    pub quota: u64,
    pub used_quota: u64,
    #[serde(default)]
    pub premium: bool,
}

// Response shapes below follow docs.pcloud.com but are parsed defensively: optional fields
// default, unknown fields are ignored, and timestamps may arrive as numbers or strings.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublicLink {
    pub link_id: u64,
    pub code: String,
    pub url: String,
    pub file_id: Option<u64>,
    pub folder_id: Option<u64>,
    pub name: String,
    pub created: String,
    pub downloads: u64,
    pub is_folder: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub id: u64,
    pub is_request: bool,
    pub incoming: bool,
    pub folder_id: Option<u64>,
    pub folder_name: String,
    pub email: String,
    pub can_modify: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UploadLink {
    pub id: u64,
    pub code: String,
    pub url: String,
    pub folder_id: u64,
    pub comment: String,
    pub expires: Option<String>,
    pub max_files: Option<u64>,
    pub files: u64,
}

#[derive(Debug, Clone)]
pub struct DiffEvent {
    pub id: u64,
    pub event: String,
    pub time: u64,
    pub metadata: Option<Metadata>,
}

/// Maximum number of events requested per change-feed page.
pub const DIFF_PAGE_SIZE: usize = 1000;

#[derive(Debug, Clone)]
pub struct DiffBatch {
    pub cursor: u64,
    pub events: Vec<DiffEvent>,
}

/// Synchronous API intended to run on a desktop blocking worker, not a UI thread.
#[derive(Clone)]
pub struct Client {
    account: Account,
    http: HttpClient,
    #[cfg(test)]
    test_api_base: Option<String>,
}

impl Client {
    pub fn new(account: Account) -> Result<Self> {
        if account.token.is_empty()
            || account.token.len() > 4096
            || account.token.chars().any(char::is_control)
        {
            return Err(Error::Authentication);
        }
        let http = HttpClient::builder()
            .user_agent("pCloud-Vault/0.1")
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            account,
            http,
            #[cfg(test)]
            test_api_base: None,
        })
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    fn endpoint(&self, method: &str) -> String {
        #[cfg(test)]
        if let Some(base) = &self.test_api_base {
            return format!("{base}/{method}");
        }
        format!("https://{}/{method}", self.account.region.api_host())
    }

    fn call(&self, method: &str, parameters: &[(&str, String)]) -> Result<Value> {
        let mut form = parameters.to_vec();
        let token_parameter = self.account.token_parameter();
        form.push((token_parameter, self.account.token.clone()));
        form.push(("timeformat", "timestamp".into()));
        let result = self.http.post(self.endpoint(method)).form(&form).send();
        for (key, value) in &mut form {
            if *key == token_parameter {
                value.zeroize();
            }
        }
        checked_response(result.map_err(|_| Error::Network)?)
    }

    pub fn user_info(&self) -> Result<UserInfo> {
        let value = self.call("userinfo", &[])?;
        Ok(UserInfo {
            user_id: required_u64(&value, "userid")?,
            email: required_str(&value, "email")?.into(),
            quota: required_u64(&value, "quota")?,
            used_quota: required_u64(&value, "usedquota")?,
            premium: value
                .get("premium")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub fn list_folder(&self, folder_id: u64) -> Result<Vec<Metadata>> {
        let value = self.call("listfolder", &[("folderid", folder_id.to_string())])?;
        value
            .get("metadata")
            .and_then(|x| x.get("contents"))
            .and_then(Value::as_array)
            .ok_or(Error::InvalidResponse)?
            .iter()
            .map(parse_metadata)
            .collect()
    }

    pub fn stat(&self, id: u64, is_folder: bool) -> Result<Metadata> {
        let value = self.call(
            if is_folder { "listfolder" } else { "stat" },
            &[(
                if is_folder { "folderid" } else { "fileid" },
                id.to_string(),
            )],
        )?;
        let metadata = parse_metadata(value.get("metadata").ok_or(Error::InvalidResponse)?)?;
        if metadata.id != id || metadata.is_folder != is_folder {
            return Err(Error::InvalidResponse);
        }
        Ok(metadata)
    }

    /// Initial reconciliation from an account subtree; the caller persists a diff cursor separately.
    pub fn list_tree(&self, folder_id: u64) -> Result<Vec<Metadata>> {
        let value = self.call(
            "listfolder",
            &[
                ("folderid", folder_id.to_string()),
                ("recursive", "1".into()),
            ],
        )?;
        let mut result = Vec::new();
        flatten_metadata(
            value.get("metadata").ok_or(Error::InvalidResponse)?,
            &mut result,
            0,
        )?;
        Ok(result)
    }

    pub fn create_folder(&self, parent: u64, name: &str) -> Result<Metadata> {
        validate_name(name)?;
        let value = self.call(
            "createfolder",
            &[("folderid", parent.to_string()), ("name", name.into())],
        )?;
        parse_metadata(value.get("metadata").ok_or(Error::InvalidResponse)?)
    }

    pub fn rename_file(&self, id: u64, name: &str) -> Result<Metadata> {
        self.rename("renamefile", "fileid", id, "toname", name)
    }
    pub fn rename_folder(&self, id: u64, name: &str) -> Result<Metadata> {
        self.rename("renamefolder", "folderid", id, "toname", name)
    }
    fn rename(
        &self,
        method: &str,
        id_key: &str,
        id: u64,
        name_key: &str,
        name: &str,
    ) -> Result<Metadata> {
        validate_name(name)?;
        let value = self.call(method, &[(id_key, id.to_string()), (name_key, name.into())])?;
        parse_metadata(value.get("metadata").ok_or(Error::InvalidResponse)?)
    }
    pub fn move_file(&self, id: u64, parent: u64) -> Result<Metadata> {
        self.move_item("renamefile", "fileid", id, parent)
    }
    pub fn move_folder(&self, id: u64, parent: u64) -> Result<Metadata> {
        self.move_item("renamefolder", "folderid", id, parent)
    }
    fn move_item(&self, method: &str, id_key: &str, id: u64, parent: u64) -> Result<Metadata> {
        let value = self.call(
            method,
            &[(id_key, id.to_string()), ("tofolderid", parent.to_string())],
        )?;
        parse_metadata(value.get("metadata").ok_or(Error::InvalidResponse)?)
    }
    /// Verifies a website session token and fills in the account identity. The `locationid`
    /// cookie normally names the region; the other region is tried once if it was wrong.
    pub fn from_web_session(token: &str, location_id: Option<u64>) -> Result<Self> {
        let region = location_id
            .map(Region::from_location_id)
            .transpose()?
            .unwrap_or(Region::Us);
        let mut last = Error::Authentication;
        for region in [region, region.other()] {
            let client = Self::new(Account {
                token: token.to_owned(),
                region,
                user_id: 0,
                email: String::new(),
                auth: AuthKind::WebSession,
            })?;
            match client.user_info() {
                Ok(info) => {
                    let mut account = client.account.clone();
                    account.user_id = info.user_id;
                    account.email = info.email;
                    return Ok(Self { account, ..client });
                }
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    /// Invalidates a website session token at pCloud. OAuth tokens are left to the user's
    /// pCloud app settings.
    pub fn logout(&self) -> Result<()> {
        if self.account.auth == AuthKind::WebSession {
            self.call("logout", &[])?;
        }
        Ok(())
    }

    pub fn delete_file(&self, id: u64) -> Result<()> {
        self.call("deletefile", &[("fileid", id.to_string())])
            .map(|_| ())
    }
    pub fn delete_folder(&self, id: u64) -> Result<()> {
        self.call("deletefolderrecursive", &[("folderid", id.to_string())])
            .map(|_| ())
    }

    pub fn restore(&self, id: u64, is_folder: bool) -> Result<()> {
        self.call(
            "trash_restore",
            &[(
                if is_folder { "folderid" } else { "fileid" },
                id.to_string(),
            )],
        )
        .map(|_| ())
    }

    pub fn list_trash(&self) -> Result<Vec<Metadata>> {
        let value = self.call("trash_list", &[])?;
        let mut result = Vec::new();
        let contents = value
            .get("trash")
            .and_then(Value::as_array)
            .or_else(|| {
                value
                    .get("metadata")
                    .and_then(|x| x.get("contents"))
                    .and_then(Value::as_array)
            })
            .ok_or(Error::InvalidResponse)?;
        for item in contents {
            result.push(parse_metadata(item)?);
        }
        Ok(result)
    }

    pub fn upload_reader<R: Read + Send + 'static>(
        &self,
        parent: u64,
        name: &str,
        reader: R,
        length: u64,
        immutable: bool,
    ) -> Result<Metadata> {
        validate_name(name)?;
        let part = multipart::Part::reader_with_length(reader, length)
            .file_name(name.to_owned())
            .mime_str("application/octet-stream")
            .map_err(|_| Error::InvalidName)?;
        let form = multipart::Form::new()
            .text(self.account.token_parameter(), self.account.token.clone())
            .text("timeformat", "timestamp")
            .text("folderid", parent.to_string())
            .text("nopartial", "1")
            .text("renameifexists", if immutable { "1" } else { "0" })
            .part("file", part);
        let response = self
            .http
            .post(self.endpoint("uploadfile"))
            .timeout(Duration::from_secs(6 * 60 * 60))
            .multipart(form)
            .send()
            .map_err(|_| Error::Network)?;
        let value = checked_response(response)?;
        parse_metadata(
            value
                .get("metadata")
                .and_then(Value::as_array)
                .and_then(|x| x.first())
                .ok_or(Error::InvalidResponse)?,
        )
    }

    /// Uploads immutable vault objects and verifies a read-back digest before publication.
    /// If an identical name already exists, its bytes must match rather than overwriting it.
    pub fn upload_bytes_immutable(
        &self,
        parent: u64,
        name: &str,
        data: Vec<u8>,
    ) -> Result<Metadata> {
        validate_name(name)?;
        let expected: [u8; 32] = Sha256::digest(&data).into();
        let length = data.len() as u64;
        if let Some(existing) = self
            .list_folder(parent)?
            .into_iter()
            .find(|item| item.name == name)
        {
            if existing.is_folder || existing.size != length {
                return Err(Error::Integrity);
            }
            self.verify_digest(existing.id, length, expected)?;
            return Ok(existing);
        }
        let uploaded = self.upload_reader(parent, name, Cursor::new(data), length, true)?;
        if uploaded.name != name {
            // A racing uploader may have created the same immutable object. Remove the
            // renamed duplicate this call created, then validate the canonical object.
            self.delete_file(uploaded.id)?;
            let canonical = self
                .list_folder(parent)?
                .into_iter()
                .find(|item| item.name == name && !item.is_folder)
                .ok_or(Error::Integrity)?;
            self.verify_digest(canonical.id, length, expected)?;
            return Ok(canonical);
        }
        self.verify_digest(uploaded.id, length, expected)?;
        Ok(uploaded)
    }

    fn verify_digest(&self, id: u64, length: u64, expected: [u8; 32]) -> Result<()> {
        let mut sink = DigestWriter(Sha256::new());
        let received = self.download_to(id, &mut sink)?;
        let actual: [u8; 32] = sink.0.finalize().into();
        if received != length || actual != expected {
            return Err(Error::Integrity);
        }
        Ok(())
    }

    pub fn download_to<W: Write>(&self, id: u64, writer: &mut W) -> Result<u64> {
        let value = self.call(
            "getfilelink",
            &[
                ("fileid", id.to_string()),
                ("forcedownload", "1".into()),
                ("skipfilename", "1".into()),
            ],
        )?;
        let url = link_url(&value)?;
        // This client has no default bearer headers or cookies. The signed URL is the sole credential.
        let mut response = self
            .http
            .get(url)
            .timeout(Duration::from_secs(6 * 60 * 60))
            .send()
            .map_err(|_| Error::Network)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        io::copy(&mut response, writer).map_err(|_| Error::Io)
    }

    /// Fetches a cropped thumbnail (for example `"256x256"`) of at most 4 MiB.
    pub fn thumbnail(&self, file_id: u64, size: &str) -> Result<Vec<u8>> {
        validate_thumbnail_size(size)?;
        let value = self.call(
            "getthumblink",
            &[
                ("fileid", file_id.to_string()),
                ("size", size.into()),
                ("crop", "1".into()),
            ],
        )?;
        let url = link_url(&value)?;
        let response = self
            .http
            .get(url)
            .timeout(Duration::from_secs(60))
            .send()
            .map_err(|_| Error::Network)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_THUMBNAIL + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Network)?;
        if bytes.len() as u64 > MAX_THUMBNAIL {
            return Err(Error::InvalidResponse);
        }
        Ok(bytes)
    }

    pub fn create_file_link(&self, file_id: u64) -> Result<PublicLink> {
        let value = self.call("getfilepublink", &[("fileid", file_id.to_string())])?;
        let mut link = parse_public_link(&value)?;
        link.file_id.get_or_insert(file_id);
        link.is_folder = false;
        Ok(link)
    }

    pub fn create_folder_link(&self, folder_id: u64) -> Result<PublicLink> {
        let value = self.call("getfolderpublink", &[("folderid", folder_id.to_string())])?;
        let mut link = parse_public_link(&value)?;
        link.folder_id.get_or_insert(folder_id);
        link.is_folder = true;
        Ok(link)
    }

    pub fn list_links(&self) -> Result<Vec<PublicLink>> {
        parse_public_links(&self.call("listpublinks", &[])?)
    }

    pub fn delete_link(&self, link_id: u64) -> Result<()> {
        self.call("deletepublink", &[("linkid", link_id.to_string())])
            .map(|_| ())
    }

    pub fn list_shares(&self) -> Result<Vec<ShareInfo>> {
        parse_shares(&self.call("listshares", &[])?)
    }

    pub fn share_folder(&self, folder_id: u64, email: &str, can_modify: bool) -> Result<()> {
        validate_email(email)?;
        self.call(
            "sharefolder",
            &[
                ("folderid", folder_id.to_string()),
                ("mail", email.into()),
                // pCloud bitmask: 1 create, 2 modify, 4 delete; 0 is view-only.
                ("permissions", if can_modify { "7" } else { "0" }.into()),
            ],
        )
        .map(|_| ())
    }

    pub fn accept_share(&self, request_id: u64) -> Result<()> {
        self.call("acceptshare", &[("sharerequestid", request_id.to_string())])
            .map(|_| ())
    }

    pub fn decline_share(&self, request_id: u64) -> Result<()> {
        self.call(
            "declineshare",
            &[("sharerequestid", request_id.to_string())],
        )
        .map(|_| ())
    }

    pub fn cancel_share_request(&self, request_id: u64) -> Result<()> {
        self.call(
            "cancelsharerequest",
            &[("sharerequestid", request_id.to_string())],
        )
        .map(|_| ())
    }

    pub fn remove_share(&self, share_id: u64) -> Result<()> {
        self.call("removeshare", &[("shareid", share_id.to_string())])
            .map(|_| ())
    }

    /// `expire` is a datetime string accepted by pCloud (for example `2030-01-31T12:00:00`).
    pub fn create_upload_link(
        &self,
        folder_id: u64,
        comment: &str,
        expire: Option<&str>,
        max_files: Option<u64>,
    ) -> Result<UploadLink> {
        validate_comment(comment)?;
        let mut parameters = vec![
            ("folderid", folder_id.to_string()),
            ("comment", comment.into()),
        ];
        if let Some(expire) = expire {
            validate_expire(expire)?;
            parameters.push(("expire", expire.into()));
        }
        if let Some(max_files) = max_files {
            parameters.push(("maxfiles", max_files.to_string()));
        }
        let value = self.call("createuploadlink", &parameters)?;
        let mut link = parse_upload_link(&value)?;
        if link.folder_id == 0 {
            link.folder_id = folder_id;
        }
        Ok(link)
    }

    pub fn list_upload_links(&self) -> Result<Vec<UploadLink>> {
        parse_upload_links(&self.call("listuploadlinks", &[])?)
    }

    pub fn delete_upload_link(&self, id: u64) -> Result<()> {
        self.call("deleteuploadlink", &[("uploadlinkid", id.to_string())])
            .map(|_| ())
    }

    /// Returns up to [`DIFF_PAGE_SIZE`] events after `cursor`, or only the current position
    /// (no events) when `cursor` is `None`.
    pub fn diff(&self, cursor: Option<u64>, block: bool) -> Result<DiffBatch> {
        let mut parameters = vec![("limit", DIFF_PAGE_SIZE.to_string())];
        if let Some(cursor) = cursor {
            parameters.push(("diffid", cursor.to_string()));
            if block {
                parameters.push(("block", "1".into()));
            }
        } else {
            parameters.push(("last", "0".into()));
        }
        parse_diff(&self.call("diff", &parameters)?)
    }
}

struct DigestWriter(Sha256);
impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn checked_response(response: Response) -> Result<Value> {
    if !response.status().is_success() {
        return Err(Error::Http(response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_API_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Network)?;
    if bytes.len() as u64 > MAX_API_RESPONSE {
        return Err(Error::InvalidResponse);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)?;
    check_result(&value)?;
    Ok(value)
}

fn check_result(value: &Value) -> Result<()> {
    match required_u64(value, "result")? {
        0 => Ok(()),
        1000 | 2000 => Err(Error::Authentication),
        2003 => Err(Error::AccessDenied),
        2002 | 2005 | 2009 => Err(Error::NotFound),
        2008 => Err(Error::Quota),
        code => Err(Error::Api(code)),
    }
}
fn required_u64(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(Error::InvalidResponse)
}
fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error::InvalidResponse)
}
fn parse_metadata(value: &Value) -> Result<Metadata> {
    let is_folder = value
        .get("isfolder")
        .and_then(Value::as_bool)
        .ok_or(Error::InvalidResponse)?;
    Ok(Metadata {
        id: required_u64(value, if is_folder { "folderid" } else { "fileid" })?,
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        is_folder,
        parent_folder_id: value
            .get("parentfolderid")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        size: value.get("size").and_then(Value::as_u64).unwrap_or(0),
        modified: value.get("modified").and_then(Value::as_u64).unwrap_or(0),
        hash: value.get("hash").and_then(Value::as_u64),
    })
}
/// Builds a validated download URL from a `hosts` + `path` link response.
fn link_url(value: &Value) -> Result<Url> {
    let host = value
        .get("hosts")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .and_then(Value::as_str)
        .ok_or(Error::InvalidResponse)?;
    validate_download_url(host, required_str(value, "path")?)
}
fn optional_string(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}
fn parse_public_link(value: &Value) -> Result<PublicLink> {
    let metadata = value.get("metadata").filter(|m| m.is_object());
    let is_folder = metadata
        .and_then(|m| m.get("isfolder"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let item_id = |key: &str| {
        metadata
            .and_then(|m| m.get(key))
            .and_then(Value::as_u64)
            .or_else(|| value.get(key).and_then(Value::as_u64))
    };
    Ok(PublicLink {
        link_id: required_u64(value, "linkid")?,
        code: optional_string(value, "code").unwrap_or_default(),
        url: optional_string(value, "link").unwrap_or_default(),
        file_id: if is_folder { None } else { item_id("fileid") },
        folder_id: if is_folder { item_id("folderid") } else { None },
        name: metadata
            .and_then(|m| m.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        created: optional_string(value, "created").unwrap_or_default(),
        downloads: value.get("downloads").and_then(Value::as_u64).unwrap_or(0),
        is_folder,
    })
}
fn parse_public_links(value: &Value) -> Result<Vec<PublicLink>> {
    value
        .get("publinks")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidResponse)?
        .iter()
        .map(parse_public_link)
        .collect()
}
fn parse_shares(value: &Value) -> Result<Vec<ShareInfo>> {
    let mut shares = Vec::new();
    for (section, is_request) in [("shares", false), ("requests", true)] {
        let Some(section) = value.get(section) else {
            continue;
        };
        for (direction, incoming) in [("incoming", true), ("outgoing", false)] {
            let Some(items) = section.get(direction) else {
                continue;
            };
            for item in items.as_array().ok_or(Error::InvalidResponse)? {
                shares.push(parse_share(item, is_request, incoming)?);
            }
        }
    }
    Ok(shares)
}
fn parse_share(value: &Value, is_request: bool, incoming: bool) -> Result<ShareInfo> {
    let flag = |key: &str| {
        value
            .get("permissions")
            .and_then(|p| p.get(key))
            .or_else(|| value.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    // Incoming entries name the sender, outgoing entries the recipient.
    let email_key = if incoming { "frommail" } else { "tomail" };
    Ok(ShareInfo {
        id: required_u64(
            value,
            if is_request {
                "sharerequestid"
            } else {
                "shareid"
            },
        )?,
        is_request,
        incoming,
        folder_id: value.get("folderid").and_then(Value::as_u64),
        folder_name: value
            .get("sharename")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        email: value
            .get(email_key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        can_modify: flag("canmodify") || flag("cancreate") || flag("candelete"),
    })
}
fn parse_upload_link(value: &Value) -> Result<UploadLink> {
    Ok(UploadLink {
        id: required_u64(value, "uploadlinkid").or_else(|_| required_u64(value, "id"))?,
        code: optional_string(value, "code").unwrap_or_default(),
        url: optional_string(value, "link").unwrap_or_default(),
        // listuploadlinks nests the target folder in `metadata`; createuploadlink may not.
        folder_id: value
            .get("folderid")
            .or_else(|| value.get("metadata").and_then(|m| m.get("folderid")))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        comment: value
            .get("comment")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        expires: optional_string(value, "expire").or_else(|| optional_string(value, "expires")),
        max_files: value
            .get("maxfiles")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0),
        files: value
            .get("files")
            .and_then(|f| f.as_u64().or_else(|| f.as_array().map(|a| a.len() as u64)))
            .unwrap_or(0),
    })
}
fn parse_upload_links(value: &Value) -> Result<Vec<UploadLink>> {
    value
        .get("links")
        .or_else(|| value.get("uploadlinks"))
        .and_then(Value::as_array)
        .ok_or(Error::InvalidResponse)?
        .iter()
        .map(parse_upload_link)
        .collect()
}
fn validate_email(email: &str) -> Result<()> {
    if email.is_empty()
        || email.len() >= 255
        || !email.contains('@')
        || email.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn validate_comment(comment: &str) -> Result<()> {
    if comment.len() >= 1000 || comment.chars().any(char::is_control) {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn validate_expire(expire: &str) -> Result<()> {
    if expire.is_empty() || expire.len() > 64 || expire.chars().any(char::is_control) {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn validate_thumbnail_size(size: &str) -> Result<()> {
    let valid = size.split_once('x').is_some_and(|(w, h)| {
        [w, h]
            .iter()
            .all(|n| !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()))
    });
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidInput)
    }
}
fn flatten_metadata(value: &Value, output: &mut Vec<Metadata>, depth: usize) -> Result<()> {
    if depth > 256 || output.len() > 500_000 {
        return Err(Error::InvalidResponse);
    }
    if let Some(contents) = value.get("contents").and_then(Value::as_array) {
        for child in contents {
            output.push(parse_metadata(child)?);
            flatten_metadata(child, output, depth + 1)?;
        }
    }
    Ok(())
}
fn parse_diff(value: &Value) -> Result<DiffBatch> {
    let events = value
        .get("entries")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidResponse)?
        .iter()
        .map(|entry| {
            Ok(DiffEvent {
                id: required_u64(entry, "diffid")?,
                event: required_str(entry, "event")?.into(),
                time: required_u64(entry, "time")?,
                metadata: entry.get("metadata").map(parse_metadata).transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(DiffBatch {
        cursor: required_u64(value, "diffid")?,
        events,
    })
}
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 1024
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return Err(Error::InvalidName);
    }
    Ok(())
}
fn validate_download_url(host: &str, path: &str) -> Result<Url> {
    if !(host == "pcloud.com" || host.ends_with(".pcloud.com"))
        || host.contains(['/', ':', '@', '\\'])
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '\r', '\n', '#'])
    {
        return Err(Error::UnsafeDownload);
    }
    let url = Url::parse(&format!("https://{host}{path}")).map_err(|_| Error::UnsafeDownload)?;
    if url.host_str() != Some(host)
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(Error::UnsafeDownload);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn regional_hosts_are_exact() {
        assert_eq!(Region::from_oauth(1, "api.pcloud.com").unwrap(), Region::Us);
        assert_eq!(
            Region::from_oauth(2, "eapi.pcloud.com").unwrap(),
            Region::Eu
        );
        for (id, host) in [
            (1, "eapi.pcloud.com"),
            (2, "api.pcloud.com"),
            (1, "api.pcloud.com.attacker.test"),
            (9, "api.pcloud.com"),
        ] {
            assert!(Region::from_oauth(id, host).is_err());
        }
    }
    #[test]
    fn download_urls_cannot_escape_pcloud() {
        assert!(validate_download_url("c63.pcloud.com", "/hash/file%20name").is_ok());
        for (host, path) in [
            ("pcloud.com.attacker.test", "/x"),
            ("c63.pcloud.com@attacker.test", "/x"),
            ("127.0.0.1", "/x"),
            ("c63.pcloud.com", "//attacker.test/x"),
            ("c63.pcloud.com", "/x#fragment"),
            ("c63.pcloud.com:443", "/x"),
        ] {
            assert!(validate_download_url(host, path).is_err());
        }
    }
    #[test]
    fn api_metadata_retains_64_bit_ids() {
        let item = parse_metadata(&json!({"isfolder":false,"fileid":u64::MAX,"name":"契約書.txt","parentfolderid":8,"size":9,"modified":123})).unwrap();
        assert_eq!(item.id, u64::MAX);
        assert_eq!(item.modified, 123);
        assert_eq!(item.name, "契約書.txt");
        assert!(parse_metadata(&json!({"isfolder":true,"folderid":-1})).is_err());
    }
    #[test]
    fn metadata_hash_is_optional() {
        let file =
            parse_metadata(&json!({"isfolder":false,"fileid":1,"hash":18446744073709551615u64}))
                .unwrap();
        assert_eq!(file.hash, Some(u64::MAX));
        let folder = parse_metadata(&json!({"isfolder":true,"folderid":2})).unwrap();
        assert_eq!(folder.hash, None);
    }
    #[test]
    fn public_links_parse_files_and_folders() {
        let links = parse_public_links(&json!({"result":0,"publinks":[
            {"linkid":11,"code":"XYZ","link":"https://u.pcloud.link/publink/show?code=XYZ","created":1700000000,"downloads":3,"extra":true,
             "metadata":{"isfolder":false,"fileid":5,"name":"a.txt"}},
            {"linkid":12,"code":"ABC","link":"https://u.pcloud.link/publink/show?code=ABC","created":"2030-01-01",
             "metadata":{"isfolder":true,"folderid":6,"name":"dir"}}]}))
        .unwrap();
        assert_eq!(links[0].file_id, Some(5));
        assert_eq!(links[0].folder_id, None);
        assert_eq!(links[0].created, "1700000000");
        assert_eq!(links[0].downloads, 3);
        assert!(links[1].is_folder);
        assert_eq!(links[1].folder_id, Some(6));
        assert_eq!(links[1].downloads, 0);
        assert!(parse_public_links(&json!({"result":0})).is_err());
        let created = parse_public_link(&json!({"result":0,"linkid":13,"code":"Q","link":"https://u.pcloud.link/publink/show?code=Q"})).unwrap();
        assert_eq!(created.file_id, None);
    }
    #[test]
    fn shares_and_requests_parse_in_both_directions() {
        let shares = parse_shares(&json!({"result":0,
            "shares":{"incoming":[{"shareid":1,"folderid":10,"sharename":"Docs","frommail":"a@example.com","permissions":{"canread":true,"canmodify":true}}],
                      "outgoing":[{"shareid":2,"folderid":11,"sharename":"Pics","tomail":"b@example.com","canread":true}]},
            "requests":{"incoming":[{"sharerequestid":3,"sharename":"Req","frommail":"c@example.com","permissions":{"cancreate":true}}],
                        "outgoing":[{"sharerequestid":4,"folderid":12,"sharename":"Out","tomail":"d@example.com"}]}})).unwrap();
        assert_eq!(shares.len(), 4);
        assert_eq!(shares[0].id, 1);
        assert!(shares[0].incoming && !shares[0].is_request && shares[0].can_modify);
        assert_eq!(shares[0].email, "a@example.com");
        assert_eq!(shares[1].email, "b@example.com");
        assert!(!shares[1].can_modify);
        assert!(shares[2].is_request && shares[2].can_modify && shares[2].folder_id.is_none());
        assert_eq!((shares[3].id, shares[3].incoming), (4, false));
        assert!(parse_shares(&json!({"result":0})).unwrap().is_empty());
        assert!(parse_shares(&json!({"shares":{"incoming":[{"folderid":1}]}})).is_err());
    }
    #[test]
    fn upload_links_parse_optional_limits() {
        let links = parse_upload_links(&json!({"result":0,"links":[
            {"uploadlinkid":7,"code":"UP1","link":"https://u.pcloud.link/puplink/show?code=UP1","folderid":9,"comment":"Send files","expires":"2030-01-01T00:00:00","maxfiles":5,"files":2},
            {"uploadlinkid":8,"code":"UP2","link":"https://u.pcloud.link/puplink/show?code=UP2","folderid":9}]}))
        .unwrap();
        assert_eq!(links[0].max_files, Some(5));
        assert_eq!(links[0].files, 2);
        assert_eq!(links[0].expires.as_deref(), Some("2030-01-01T00:00:00"));
        assert_eq!(links[1].max_files, None);
        assert_eq!(links[1].expires, None);
        assert_eq!(links[1].files, 0);
        assert!(parse_upload_links(&json!({"result":0})).is_err());
    }
    #[test]
    fn share_and_link_inputs_are_validated() {
        assert!(validate_email("user@example.com").is_ok());
        let long = format!("{}@example.com", "a".repeat(260));
        for bad in ["", "no-at-sign", "a@example.com\n", long.as_str()] {
            assert!(validate_email(bad).is_err());
        }
        assert!(validate_comment("").is_ok());
        assert!(validate_comment("ok").is_ok());
        assert!(validate_comment("bad\ttab").is_err());
        assert!(validate_comment(&"a".repeat(1000)).is_err());
        assert!(validate_thumbnail_size("256x256").is_ok());
        for bad in ["256", "x", "256x", "a x1", "256x256&crop=0"] {
            assert!(validate_thumbnail_size(bad).is_err());
        }
    }
    #[test]
    fn thumbnail_links_use_the_download_host_allow_list() {
        let ok = json!({"hosts":["c1.pcloud.com"],"path":"/thumb/x.jpg"});
        assert!(link_url(&ok).is_ok());
        let evil = json!({"hosts":["attacker.test"],"path":"/thumb/x.jpg"});
        assert!(matches!(link_url(&evil), Err(Error::UnsafeDownload)));
    }
    #[test]
    fn share_folder_sends_permission_bitmask() {
        let (client, request) = mock_client(json!({"result":0}).to_string());
        client.share_folder(7, "user@example.com", true).unwrap();
        let text = String::from_utf8(request.join().unwrap()).unwrap();
        assert!(text.starts_with("POST /sharefolder HTTP/1.1"));
        assert!(text.contains("permissions=7"));
        assert!(text.contains("mail=user%40example.com"));
    }
    #[test]
    fn errors_never_echo_server_content_or_tokens() {
        let error = check_result(&json!({"result":2003,"error":"SECRET_TOKEN"})).unwrap_err();
        assert!(!error.to_string().contains("SECRET_TOKEN"));
        let account = Account {
            token: "SECRET_TOKEN".into(),
            region: Region::Eu,
            user_id: 1,
            email: "secret@example.com".into(),
            auth: AuthKind::OAuth,
        };
        assert!(!format!("{account:?}").contains("SECRET_TOKEN"));
    }
    #[test]
    fn diff_events_parse_with_a_nonconsecutive_cursor() {
        let batch = parse_diff(&json!({"result":0,"diffid":90,"entries":[{"diffid":89,"time":20,"event":"deletefile","metadata":{"isfolder":false,"fileid":7,"name":"old.txt"}}]})).unwrap();
        assert_eq!(batch.cursor, 90);
        assert_eq!(batch.events[0].metadata.as_ref().unwrap().id, 7);
    }

    #[test]
    fn immutable_upload_posts_token_and_no_partial_flags_in_body() {
        let response = json!({"result":0,"metadata":[{"isfolder":false,"fileid":9,"name":"opaque-object","parentfolderid":7,"size":3,"modified":123}]}).to_string();
        let (client, request) = mock_client(response);
        let metadata = client
            .upload_reader(7, "opaque-object", Cursor::new(b"abc".to_vec()), 3, true)
            .unwrap();
        assert_eq!(metadata.id, 9);
        let bytes = request.join().unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let (headers, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /uploadfile HTTP/1.1"));
        assert!(!headers.contains("test-secret-token"));
        for marker in [
            "name=\"access_token\"\r\n\r\ntest-secret-token",
            "name=\"nopartial\"\r\n\r\n1",
            "name=\"renameifexists\"\r\n\r\n1",
        ] {
            assert!(body.contains(marker));
        }
    }

    #[test]
    fn website_sessions_send_the_auth_parameter_and_can_log_out() {
        let (client, request) = mock_client_as(
            json!({"result":0,"userid":5,"email":"user@example.com","quota":10,"usedquota":1})
                .to_string(),
            AuthKind::WebSession,
        );
        assert_eq!(client.user_info().unwrap().user_id, 5);
        let text = String::from_utf8(request.join().unwrap()).unwrap();
        assert!(text.contains("auth=test-secret-token"));
        assert!(!text.contains("access_token"));
        let (client, request) =
            mock_client_as(json!({"result":0}).to_string(), AuthKind::WebSession);
        client.logout().unwrap();
        let text = String::from_utf8(request.join().unwrap()).unwrap();
        assert!(text.starts_with("POST /logout HTTP/1.1"));
        assert_eq!(Region::from_location_id(2).unwrap(), Region::Eu);
        assert!(Region::from_location_id(3).is_err());
    }

    #[test]
    fn stat_routes_folder_and_file_ids_without_query_credentials() {
        let (client,request) = mock_client(json!({"result":0,"metadata":{"isfolder":true,"folderid":7,"name":"folder","parentfolderid":1}}).to_string());
        assert_eq!(client.stat(7, true).unwrap().parent_folder_id, 1);
        let text = String::from_utf8(request.join().unwrap()).unwrap();
        assert!(text.starts_with("POST /listfolder HTTP/1.1"));
        assert!(text.contains("folderid=7"));
        assert!(text.contains("access_token=test-secret-token"));
        let (client,request) = mock_client(json!({"result":0,"metadata":{"isfolder":false,"fileid":8,"name":"file","parentfolderid":7}}).to_string());
        assert_eq!(client.stat(8, false).unwrap().id, 8);
        assert!(String::from_utf8(request.join().unwrap())
            .unwrap()
            .starts_with("POST /stat HTTP/1.1"));
    }

    fn mock_client(response: String) -> (Client, std::thread::JoinHandle<Vec<u8>>) {
        mock_client_as(response, AuthKind::OAuth)
    }

    fn mock_client_as(
        response: String,
        auth: AuthKind,
    ) -> (Client, std::thread::JoinHandle<Vec<u8>>) {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= header_end + 4 + length {
                        break;
                    }
                }
                assert!(bytes.len() < 65536);
            }
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            bytes
        });
        let account = Account {
            token: "test-secret-token".into(),
            region: Region::Eu,
            user_id: 1,
            email: "test@example.invalid".into(),
            auth,
        };
        (
            Client {
                account,
                http: HttpClient::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap(),
                test_api_base: Some(base),
            },
            thread,
        )
    }
}
