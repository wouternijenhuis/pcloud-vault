//! Known-answer tests that pin the on-disk vault format. Dependency upgrades must keep
//! every expected value here unchanged: a different value means existing vaults break.

use super::crypto::{derive_key, derive_password, VaultHeader};
use super::sync::derived_id;
use super::*;
use std::path::{Path, PathBuf};

const FIXTURE_PASSWORD: &str = "synthetic fixture passphrase";
const FIXTURE_FOLDER: &str = "Fixture folder";
const FIXTURE_FILE: &str = "fixture résumé.txt";
const FIXTURE_CONTENTS: &[u8] = b"synthetic contents that must stay readable";

fn test_kdf() -> KdfParams {
    KdfParams {
        memory_kib: 8 * 1024,
        iterations: 3,
        lanes: 1,
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v1")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for item in fs::read_dir(from).unwrap() {
        let item = item.unwrap();
        let target = to.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            copy_tree(&item.path(), &target);
        } else {
            fs::copy(item.path(), target).unwrap();
        }
    }
}

#[test]
fn argon2id_password_derivation_is_stable() {
    let header = VaultHeader {
        format_version: FORMAT_VERSION,
        vault_id: "0123456789abcdef0123456789abcdef".into(),
        kdf: test_kdf(),
        salt: "000102030405060708090a0b0c0d0e0f".into(),
        wrapped_key: String::new(),
        key_check: String::new(),
    };
    assert_eq!(
        hex::encode(derive_password(FIXTURE_PASSWORD, &header).unwrap()),
        "23895aa891d557857f8584e09a3075e6bedf683c28b301307e6963bb6bda04e5"
    );
}

#[test]
fn hkdf_key_derivation_is_stable() {
    assert_eq!(
        hex::encode(derive_key(&[0x42; 32], b"pcloud-vault/test-domain")),
        "48c4bd32ae9b489a93a0e0d9436bf7a2de00452680885d77e76d75f8f3abfa2b"
    );
}

#[test]
fn derived_ids_are_stable() {
    assert_eq!(
        derived_id("0123456789abcdef0123456789abcdef", "conflict", "rev-1"),
        "08a917afb1479f0fa488e635aaa5f1fc"
    );
}

#[test]
fn committed_fixture_vault_still_opens_and_decrypts() {
    let directory = tempfile::tempdir().unwrap();
    copy_tree(&fixture_dir(), directory.path());
    let vault = Vault::open(directory.path(), FIXTURE_PASSWORD).unwrap();
    let entries = vault.all_entries();
    let folder = entries
        .iter()
        .find(|entry| entry.name == FIXTURE_FOLDER)
        .expect("fixture folder");
    let file = entries
        .iter()
        .find(|entry| entry.name == FIXTURE_FILE)
        .expect("fixture file");
    assert_eq!(file.parent_id.as_deref(), Some(folder.id.as_str()));
    let mut bytes = vec![0_u8; file.size as usize];
    assert_eq!(
        vault.read(&file.id, 0, &mut bytes).unwrap(),
        FIXTURE_CONTENTS.len()
    );
    assert_eq!(bytes, FIXTURE_CONTENTS);
    assert!(Vault::open(directory.path(), "wrong passphrase").is_err());
}

/// Recreates the committed fixture. Only run this on purpose, with the crate versions that
/// wrote the vaults people already have: `cargo test -p vault-core regenerate -- --ignored`.
#[test]
#[ignore = "rewrites tests/fixtures/vault-v1"]
fn regenerate_fixture_vault() {
    let target = fixture_dir();
    let _ = fs::remove_dir_all(&target);
    let mut vault = Vault::create_with_kdf(&target, FIXTURE_PASSWORD, test_kdf()).unwrap();
    let folder = vault.create_folder(None, FIXTURE_FOLDER).unwrap();
    vault
        .import_file(
            Some(&folder.id),
            FIXTURE_FILE,
            std::io::Cursor::new(FIXTURE_CONTENTS),
        )
        .unwrap();
}
