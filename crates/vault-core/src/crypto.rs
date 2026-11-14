use crate::{Error, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub lanes: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: 256 * 1024,
            iterations: 3,
            lanes: 1,
        }
    }
}

impl KdfParams {
    fn validate(&self) -> Result<()> {
        let minimum = if cfg!(test) { 8 * 1024 } else { 64 * 1024 };
        if !(minimum..=512 * 1024).contains(&self.memory_kib)
            || !(3..=10).contains(&self.iterations)
            || !(1..=8).contains(&self.lanes)
        {
            return Err(Error::InvalidFormat(
                "unsafe or excessive password derivation parameters".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VaultHeader {
    pub format_version: u32,
    pub vault_id: String,
    pub kdf: KdfParams,
    pub salt: String,
    pub wrapped_key: String,
    pub key_check: String,
}

impl VaultHeader {
    pub fn validate(&self) -> Result<()> {
        if self.format_version != FORMAT_VERSION || !valid_id(&self.vault_id) {
            return Err(Error::InvalidFormat("unsupported vault header".into()));
        }
        self.kdf.validate()?;
        if decode_fixed::<16>(&self.salt).is_err()
            || self.wrapped_key.len() != 144
            || self.key_check.len() != 112
        {
            return Err(Error::InvalidFormat("invalid key envelope".into()));
        }
        Ok(())
    }

    fn aad(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&(
            "pcloud-vault/key-envelope",
            self.format_version,
            &self.vault_id,
            &self.kdf,
            &self.salt,
        ))?)
    }

    fn key_check_aad(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec(&(
            "pcloud-vault/header-authentication",
            self.format_version,
            &self.vault_id,
            &self.kdf,
            &self.salt,
            &self.wrapped_key,
        ))?)
    }

    pub(crate) fn new(password: &str, key: &[u8; 32], kdf: KdfParams) -> Result<Self> {
        let mut header = Self {
            format_version: FORMAT_VERSION,
            vault_id: new_id(),
            kdf,
            salt: hex::encode(random::<16>()),
            wrapped_key: String::new(),
            key_check: String::new(),
        };
        header.rewrap(password, key)?;
        Ok(header)
    }

    pub(crate) fn unwrap(&self, password: &str) -> Result<Zeroizing<[u8; 32]>> {
        self.validate()?;
        let key = derive_password(password, self)?;
        let raw = Zeroizing::new(open(
            &key,
            &hex::decode(&self.wrapped_key).map_err(|_| Error::Authentication)?,
            &self.aad()?,
        )?);
        Ok(Zeroizing::new(
            raw.as_slice()
                .try_into()
                .map_err(|_| Error::Authentication)?,
        ))
    }

    pub(crate) fn verify_key(&self, key: &[u8; 32]) -> Result<()> {
        self.validate()?;
        let plaintext = open(
            key,
            &hex::decode(&self.key_check).map_err(|_| Error::Authentication)?,
            &self.key_check_aad()?,
        )?;
        if plaintext != b"vault-key-check!" {
            return Err(Error::Authentication);
        }
        Ok(())
    }

    pub(crate) fn rewrap(&mut self, password: &str, key: &[u8; 32]) -> Result<()> {
        if password.is_empty() {
            return Err(Error::InvalidInput("password must not be empty".into()));
        }
        self.kdf.validate()?;
        self.salt = hex::encode(random::<16>());
        let wrapping_key = derive_password(password, self)?;
        self.wrapped_key = hex::encode(seal(&wrapping_key, key, &self.aad()?)?);
        self.key_check = hex::encode(seal(key, b"vault-key-check!", &self.key_check_aad()?)?);
        Ok(())
    }
}

fn derive_password(password: &str, header: &VaultHeader) -> Result<Zeroizing<[u8; 32]>> {
    header.kdf.validate()?;
    let params = Params::new(
        header.kdf.memory_kib,
        header.kdf.iterations,
        header.kdf.lanes,
        Some(32),
    )
    .map_err(|_| Error::InvalidFormat("invalid password parameters".into()))?;
    let mut output = Zeroizing::new([0_u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(
            password.as_bytes(),
            &decode_fixed::<16>(&header.salt)?,
            output.as_mut(),
        )
        .map_err(|_| Error::Authentication)?;
    Ok(output)
}

pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

pub(crate) fn new_id() -> String {
    hex::encode(random::<16>())
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn decode_fixed<const N: usize>(value: &str) -> Result<[u8; N]> {
    hex::decode(value)
        .map_err(|_| Error::InvalidFormat("invalid hexadecimal data".into()))?
        .try_into()
        .map_err(|_| Error::InvalidFormat("invalid hexadecimal length".into()))
}

pub(crate) fn derive_key(root: &[u8; 32], domain: &[u8]) -> [u8; 32] {
    let mut key = [0_u8; 32];
    Hkdf::<Sha256>::new(Some(b"pcloud-vault/v1"), root)
        .expand(domain, &mut key)
        .expect("32 byte HKDF output is supported");
    key
}

pub(crate) fn seal(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let nonce = random::<24>();
    let ciphertext = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| Error::Authentication)?;
    let mut envelope = Vec::with_capacity(24 + ciphertext.len());
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

pub(crate) fn open(key: &[u8; 32], envelope: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    if envelope.len() < 40 {
        return Err(Error::Authentication);
    }
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&envelope[..24]),
            Payload {
                msg: &envelope[24..],
                aad,
            },
        )
        .map_err(|_| Error::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_xchacha20_poly1305_vector() {
        let key: [u8; 32] = (0x80..=0x9f).collect::<Vec<_>>().try_into().unwrap();
        let nonce = hex::decode("404142434445464748494a4b4c4d4e4f5051525354555657").unwrap();
        let aad = hex::decode("50515253c0c1c2c3c4c5c6c7").unwrap();
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let ciphertext = XChaCha20Poly1305::new((&key).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .unwrap();
        assert_eq!(hex::encode(ciphertext), "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b4522f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff921f9664c97637da9768812f615c68b13b52ec0875924c1c7987947deafd8780acf49");
    }

    #[test]
    fn envelope_rejects_changed_nonce_aad_and_truncation() {
        let key = random::<32>();
        let mut ciphertext = seal(&key, b"secret", b"bound context").unwrap();
        assert!(open(&key, &ciphertext, b"other context").is_err());
        ciphertext[0] ^= 1;
        assert!(open(&key, &ciphertext, b"bound context").is_err());
        assert!(open(&key, &ciphertext[..30], b"bound context").is_err());
    }
}
