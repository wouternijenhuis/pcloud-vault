//! Current-user DPAPI only. Saved secrets cannot be decrypted by a different
//! Windows account; copying an encrypted record to another PC is not enrollment.

use crate::{Error, Result};

#[cfg(windows)]
pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    transform(data, true)
}

#[cfg(windows)]
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    transform(data, false)
}

#[cfg(not(windows))]
pub fn protect(_: &[u8]) -> Result<Vec<u8>> {
    Err(Error::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub fn unprotect(_: &[u8]) -> Result<Vec<u8>> {
    Err(Error::UnsupportedPlatform)
}

#[cfg(windows)]
fn transform(data: &[u8], protect: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    use zeroize::Zeroize;
    if data.is_empty() || data.len() > 1024 * 1024 {
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn current_user_roundtrip_and_tamper_rejection() {
        let encrypted = protect(b"private local test secret").unwrap();
        assert!(!encrypted
            .windows(b"private local test secret".len())
            .any(|window| window == b"private local test secret"));
        assert_eq!(unprotect(&encrypted).unwrap(), b"private local test secret");
        let mut altered = encrypted;
        let end = altered.len() - 1;
        altered[end] ^= 1;
        assert!(unprotect(&altered).is_err());
    }
}
