//! Protects small secrets (the pCloud sign-in, the remembered vault key) so they can only
//! be read back by the same user on the same computer.
//!
//! - Windows: current-user DPAPI. A different Windows account cannot decrypt the record,
//!   and copying it to another PC is not enrollment.
//! - macOS and Linux: a random 256-bit device key lives in the system keyring (Keychain,
//!   or a Secret Service such as GNOME Keyring/KWallet). Records are sealed with
//!   XChaCha20-Poly1305 under that key, so a copied record is useless without the keyring.

use crate::{Error, Result};

const MAX_SECRET_LEN: usize = 1024 * 1024;

#[cfg(windows)]
pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    dpapi(data, true)
}

#[cfg(windows)]
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    dpapi(data, false)
}

#[cfg(not(windows))]
pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    let key = keyring_store::device_key(true)?;
    seal(&key, data)
}

#[cfg(not(windows))]
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    let key = keyring_store::device_key(false)?;
    open(&key, data)
}

const SEAL_VERSION: u8 = 1;
const NONCE_LEN: usize = 24;

/// Sealed record: version byte, random 192-bit nonce, then ciphertext with its tag.
#[cfg_attr(windows, allow(dead_code))]
fn seal(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    use chacha20poly1305::{
        aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
        XChaCha20Poly1305,
    };
    if data.is_empty() || data.len() > MAX_SECRET_LEN {
        return Err(Error::DeviceProtection);
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let sealed = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: data,
                aad: &[SEAL_VERSION],
            },
        )
        .map_err(|_| Error::DeviceProtection)?;
    let mut record = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
    record.push(SEAL_VERSION);
    record.extend_from_slice(&nonce);
    record.extend_from_slice(&sealed);
    Ok(record)
}

#[cfg_attr(windows, allow(dead_code))]
fn open(key: &[u8; 32], record: &[u8]) -> Result<Vec<u8>> {
    use chacha20poly1305::{
        aead::{Aead, KeyInit, Payload},
        XChaCha20Poly1305, XNonce,
    };
    if record.len() <= 1 + NONCE_LEN || record.len() > MAX_SECRET_LEN + 64 {
        return Err(Error::DeviceProtection);
    }
    if record[0] != SEAL_VERSION {
        return Err(Error::DeviceProtection);
    }
    let (nonce, sealed) = record[1..].split_at(NONCE_LEN);
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &[SEAL_VERSION],
            },
        )
        .map_err(|_| Error::DeviceProtection)
}

#[cfg(not(windows))]
mod keyring_store {
    use crate::{Error, Result};
    use zeroize::Zeroizing;

    const SERVICE: &str = "dev.pcloudvault.desktop";
    const ACCOUNT: &str = "device-key";

    /// Reads the device key, creating it on first use when `create` is set.
    pub(super) fn device_key(create: bool) -> Result<Zeroizing<[u8; 32]>> {
        // Serializes first-use creation so two concurrent callers cannot each store a
        // different key and leave the first sealed record undecryptable.
        static CREATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = CREATE.lock().map_err(|_| Error::DeviceProtection)?;
        let entry = keyring::Entry::new(SERVICE, ACCOUNT).map_err(unavailable)?;
        match entry.get_secret() {
            Ok(secret) => {
                let secret = Zeroizing::new(secret);
                let key: [u8; 32] = secret
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::DeviceProtection)?;
                Ok(Zeroizing::new(key))
            }
            Err(keyring::Error::NoEntry) if create => {
                let mut key = Zeroizing::new([0u8; 32]);
                rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key[..]);
                entry.set_secret(&key[..]).map_err(unavailable)?;
                // Use what the keyring actually stored, in case another process won.
                let stored = Zeroizing::new(entry.get_secret().map_err(unavailable)?);
                let stored: [u8; 32] = stored
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::DeviceProtection)?;
                Ok(Zeroizing::new(stored))
            }
            Err(keyring::Error::NoEntry) => Err(Error::DeviceProtection),
            Err(error) => Err(unavailable(error)),
        }
    }

    fn unavailable(error: keyring::Error) -> Error {
        match error {
            keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_) => {
                Error::KeyringUnavailable
            }
            _ => Error::DeviceProtection,
        }
    }
}

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    use zeroize::Zeroize;
    if data.is_empty() || data.len() > MAX_SECRET_LEN {
        return Err(Error::DeviceProtection);
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).map_err(|_| Error::DeviceProtection)?,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: DPAPI reads the borrowed input during the call and initializes an
    // independently allocated output blob, released with LocalFree below.
    let success = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 || output.pbData.is_null() {
        return Err(Error::DeviceProtection);
    }
    // SAFETY: the successful DPAPI result owns cbData bytes. Copy before clearing
    // the Windows allocation, and never retain a pointer into freed storage.
    let result = unsafe {
        let bytes = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = bytes.to_vec();
        bytes.zeroize();
        LocalFree(output.pbData.cast());
        result
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"private local test secret";

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[test]
    fn sealed_records_roundtrip_and_reject_tampering() {
        let key = [7u8; 32];
        let record = seal(&key, SECRET).unwrap();
        assert!(!contains(&record, SECRET));
        assert_eq!(open(&key, &record).unwrap(), SECRET);
        assert_ne!(seal(&key, SECRET).unwrap(), record, "nonces must be fresh");
        for index in [0, 1, record.len() - 1] {
            let mut altered = record.clone();
            altered[index] ^= 1;
            assert!(open(&key, &altered).is_err());
        }
        assert!(open(&[8u8; 32], &record).is_err());
        assert!(open(&key, &record[..10]).is_err());
        assert!(seal(&key, b"").is_err());
    }

    /// A record sealed by an earlier release must keep opening after dependency upgrades.
    #[test]
    fn records_sealed_by_earlier_releases_still_open() {
        let record = hex::decode("01d0667ead2dafd0702b5ac37778b06da763d7d05310234929c75b16b0ffb43e98c4b7d7313d9e118874164f9c479a6566dcb1961c855640109d85dacf7b00aeb79d").unwrap();
        assert_eq!(open(&[7u8; 32], &record).unwrap(), SECRET);
    }

    #[cfg(windows)]
    #[test]
    fn current_user_roundtrip_and_tamper_rejection() {
        let encrypted = protect(SECRET).unwrap();
        assert!(!contains(&encrypted, SECRET));
        assert_eq!(unprotect(&encrypted).unwrap(), SECRET);
        let mut altered = encrypted;
        let end = altered.len() - 1;
        altered[end] ^= 1;
        assert!(unprotect(&altered).is_err());
    }
}
