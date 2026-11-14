use crate::{
    crypto::{derive_key, new_id, open, seal, valid_id},
    Error, ReplicationObject, Result,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_METADATA_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn read_limited(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > maximum {
        return Err(Error::InvalidFormat("object is too large".into()));
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(Error::InvalidFormat("object is too large".into()));
    }
    Ok(bytes)
}

pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Renames a file and flushes the directory change before returning, so an acknowledged
/// object or checkpoint survives power loss. The destination must not exist.
pub(crate) fn durable_rename(from: &Path, to: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
        };
        let source: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both buffers are NUL-terminated UTF-16 paths that outlive the call.
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF))?;
    }
    #[cfg(not(windows))]
    {
        fs::rename(from, to)?;
        if let Some(parent) = to.parent() {
            File::open(parent)?.sync_all()?;
        }
    }
    Ok(())
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| Error::InvalidInput("missing parent directory".into()))?;
    fs::create_dir_all(directory)?;
    let staging = directory.join(format!("{}.tmp", new_id()));
    write_new(&staging, bytes)?;
    // Windows rename does not replace a destination; retain the previous encrypted checkpoint
    // until its replacement is durable and recover it if a crash interrupts publication.
    let backup = path.with_extension("previous");
    if path.exists() {
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        durable_rename(path, &backup)?;
    }
    if let Err(error) = durable_rename(&staging, path) {
        if backup.exists() && !path.exists() {
            let _ = durable_rename(&backup, path);
        }
        return Err(error);
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

pub(crate) fn recover_checkpoint(path: &Path) -> Result<()> {
    let previous = path.with_extension("previous");
    if !path.exists() && previous.exists() {
        durable_rename(&previous, path)?;
    }
    Ok(())
}

pub(crate) struct ObjectStore {
    pub root: PathBuf,
    pub kinds: BTreeMap<String, String>,
    vault_id: String,
    metadata_key: Zeroizing<[u8; 32]>,
}

impl ObjectStore {
    pub fn open(root: &Path, vault_id: &str, key: &[u8; 32]) -> Result<Self> {
        fs::create_dir_all(root.join("objects"))?;
        let mut kinds = BTreeMap::new();
        for file in fs::read_dir(root.join("objects"))? {
            let file = file?;
            let filename = file.file_name().to_string_lossy().into_owned();
            let Some(id) = filename.strip_suffix(".obj") else {
                continue;
            };
            if !valid_id(id) {
                continue;
            }
            let mut kind = [0_u8];
            File::open(file.path())?.read_exact(&mut kind)?;
            kinds.insert(id.to_string(), kind_name(kind[0])?.to_string());
        }
        Ok(Self {
            root: root.to_path_buf(),
            kinds,
            vault_id: vault_id.to_string(),
            metadata_key: Zeroizing::new(derive_key(key, b"metadata")),
        })
    }

    pub fn object(&self, id: &str) -> Result<ReplicationObject> {
        if !valid_id(id) {
            return Err(Error::InvalidInput("invalid object id".into()));
        }
        let kind = self
            .kinds
            .get(id)
            .ok_or_else(|| Error::OfflineUnavailable(id.into()))?;
        let maximum = if kind == "chunk" {
            crate::CHUNK_SIZE as u64 + 41
        } else {
            MAX_METADATA_BYTES
        };
        let bytes = read_limited(
            &self.root.join("objects").join(format!("{id}.obj")),
            maximum,
        )?;
        if bytes.is_empty() || kind_name(bytes[0])? != kind {
            return Err(Error::Authentication);
        }
        Ok(ReplicationObject {
            id: id.into(),
            kind: kind.clone(),
            data: bytes[1..].to_vec(),
        })
    }

    pub fn insert(&mut self, object: &ReplicationObject) -> Result<()> {
        if !valid_id(&object.id) || object.data.len() < 40 {
            return Err(Error::InvalidFormat("invalid object envelope".into()));
        }
        let tag = kind_tag(&object.kind)?;
        let maximum = if object.kind == "chunk" {
            crate::CHUNK_SIZE + 40
        } else {
            MAX_METADATA_BYTES as usize - 1
        };
        if object.data.len() > maximum {
            return Err(Error::InvalidFormat("object is too large".into()));
        }
        if self.kinds.contains_key(&object.id) {
            let existing = self.object(&object.id)?;
            if existing.kind != object.kind || existing.data != object.data {
                return Err(Error::Authentication);
            }
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(object.data.len() + 1);
        bytes.push(tag);
        bytes.extend_from_slice(&object.data);
        let directory = self.root.join("objects");
        let staging = directory.join(format!("{}.tmp", new_id()));
        write_new(&staging, &bytes)?;
        durable_rename(&staging, &directory.join(format!("{}.obj", object.id)))?;
        self.kinds.insert(object.id.clone(), object.kind.clone());
        Ok(())
    }

    pub fn put_metadata<T: Serialize>(&mut self, kind: &str, id: &str, value: &T) -> Result<()> {
        let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
        let object = ReplicationObject {
            id: id.into(),
            kind: kind.into(),
            data: seal(&self.metadata_key, &plaintext, &self.aad(kind, id))?,
        };
        self.insert(&object)
    }

    pub fn metadata<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<T> {
        let object = self.object(id)?;
        if object.kind != kind {
            return Err(Error::Authentication);
        }
        let plaintext =
            Zeroizing::new(open(&self.metadata_key, &object.data, &self.aad(kind, id))?);
        Ok(serde_json::from_slice(&plaintext)?)
    }

    pub fn validate_metadata(&self, object: &ReplicationObject) -> Result<()> {
        if object.kind != "chunk" {
            let _plaintext = Zeroizing::new(open(
                &self.metadata_key,
                &object.data,
                &self.aad(&object.kind, &object.id),
            )?);
        }
        Ok(())
    }

    fn aad(&self, kind: &str, id: &str) -> Vec<u8> {
        format!("pcloud-vault/v1/{}/{kind}/{id}", self.vault_id).into_bytes()
    }
}

fn kind_tag(kind: &str) -> Result<u8> {
    match kind {
        "chunk" => Ok(1),
        "manifest" => Ok(2),
        "commit" => Ok(3),
        _ => Err(Error::InvalidFormat("unknown object kind".into())),
    }
}
fn kind_name(tag: u8) -> Result<&'static str> {
    match tag {
        1 => Ok("chunk"),
        2 => Ok("manifest"),
        3 => Ok("commit"),
        _ => Err(Error::InvalidFormat("unknown object kind".into())),
    }
}
