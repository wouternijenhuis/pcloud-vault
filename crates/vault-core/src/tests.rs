use super::*;
use std::io::Cursor;
use tempfile::TempDir;

fn fixture() -> (TempDir, Vault) {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::create_with_kdf(
        directory.path(),
        "long test passphrase",
        KdfParams {
            memory_kib: 8 * 1024,
            iterations: 3,
            lanes: 1,
        },
    )
    .unwrap();
    (directory, vault)
}
fn contents(vault: &Vault, id: &str) -> Vec<u8> {
    let size = vault.entry(id).unwrap().size;
    let mut bytes = vec![0_u8; size as usize];
    assert_eq!(vault.read(id, 0, &mut bytes).unwrap(), size as usize);
    bytes
}
fn replica(source: &Vault) -> (TempDir, Vault) {
    let directory = tempfile::tempdir().unwrap();
    let mut vault =
        Vault::attach_with_key(directory.path(), source.header(), source.key_bytes()).unwrap();
    vault
        .ingest_objects(source.export_objects().unwrap())
        .unwrap();
    (directory, vault)
}
fn snapshot(vault: &Vault) -> Vec<(String, String, Option<String>, bool, u64)> {
    vault
        .all_entries()
        .iter()
        .map(|entry| {
            (
                entry.id.clone(),
                entry.name.clone(),
                entry.parent_id.clone(),
                entry.trashed,
                entry.size,
            )
        })
        .collect()
}

#[test]
fn password_recovery_and_rewrap_preserve_all_content() {
    let (directory, mut vault) = fixture();
    let file = vault
        .import_file(
            None,
            "秘密 résumé.txt",
            Cursor::new(b"confidential contents"),
        )
        .unwrap();
    let key = vault.recovery_key();
    let old_ciphertext = vault
        .object_ids()
        .iter()
        .map(|(id, _)| vault.object(id).unwrap().data)
        .collect::<Vec<_>>();
    vault
        .change_password("long test passphrase", "a different strong passphrase")
        .unwrap();
    assert_eq!(
        old_ciphertext,
        vault
            .object_ids()
            .iter()
            .map(|(id, _)| vault.object(id).unwrap().data)
            .collect::<Vec<_>>()
    );
    drop(vault);
    assert!(matches!(
        Vault::open(directory.path(), "long test passphrase"),
        Err(Error::Authentication)
    ));
    assert!(matches!(
        Vault::open(directory.path(), "wrong"),
        Err(Error::Authentication)
    ));
    let vault = Vault::open(directory.path(), "a different strong passphrase").unwrap();
    assert_eq!(contents(&vault, &file.id), b"confidential contents");
    drop(vault);
    let vault = Vault::open_with_recovery(directory.path(), &key).unwrap();
    assert_eq!(contents(&vault, &file.id), b"confidential contents");
    let existing = directory.path().join("existing.txt");
    fs::write(&existing, b"keep").unwrap();
    assert!(vault.export_file(&file.id, &existing).is_err());
    assert!(vault.export_recovery(&existing).is_err());
    assert_eq!(fs::read(existing).unwrap(), b"keep");
}

#[test]
fn malicious_header_is_bounded_before_derivation() {
    let (_directory, vault) = fixture();
    let mut header = vault.header().clone();
    header.kdf.memory_kib = u32::MAX;
    assert!(matches!(
        header.unwrap("anything"),
        Err(Error::InvalidFormat(_))
    ));
    header = vault.header().clone();
    header.format_version = 99;
    assert!(header.validate().is_err());
    header = vault.header().clone();
    header.salt = "../unsafe".into();
    assert!(header.validate().is_err());
    assert!(Vault::from_key(vault.root(), [0_u8; 32]).is_err());
}

#[test]
fn random_access_spans_chunks_and_truncation_zero_fills() {
    let (_directory, mut vault) = fixture();
    let original = vec![0x62_u8; CHUNK_SIZE + 17];
    let file = vault
        .import_file(None, "large.dat", Cursor::new(&original))
        .unwrap();
    vault
        .write(&file.id, CHUNK_SIZE as u64 - 3, b"ABCDEFGH")
        .unwrap();
    let mut expected = original;
    expected[CHUNK_SIZE - 3..CHUNK_SIZE + 5].copy_from_slice(b"ABCDEFGH");
    assert_eq!(contents(&vault, &file.id), expected);
    vault.truncate(&file.id, CHUNK_SIZE as u64 + 5).unwrap();
    expected.truncate(CHUNK_SIZE + 5);
    assert_eq!(contents(&vault, &file.id), expected);
    vault.truncate(&file.id, CHUNK_SIZE as u64 + 15).unwrap();
    expected.resize(CHUNK_SIZE + 15, 0);
    assert_eq!(contents(&vault, &file.id), expected);
    vault.write(&file.id, CHUNK_SIZE as u64 + 20, b"Z").unwrap();
    expected.resize(CHUNK_SIZE + 21, 0);
    expected[CHUNK_SIZE + 20] = b'Z';
    assert_eq!(contents(&vault, &file.id), expected);
}

#[test]
fn encrypted_objects_and_checkpoint_contain_no_plaintext_markers() {
    let (directory, mut vault) = fixture();
    let filename = "VeryDistinctSecretFileName.txt";
    let content = b"ExtremelyDistinctPrivateDocumentContents";
    let folder = vault
        .create_folder(None, "DistinctSecretFolderName")
        .unwrap();
    let file = vault
        .import_file(Some(&folder.id), filename, Cursor::new(content))
        .unwrap();
    vault.set_favorite(&file.id, true).unwrap();
    vault.set_pinned(&folder.id, true).unwrap();
    for path in [
        directory.path().to_path_buf(),
        directory.path().join("objects"),
    ] {
        for file in fs::read_dir(path).unwrap() {
            let file = file.unwrap();
            if !file.file_type().unwrap().is_file() {
                continue;
            }
            if file.file_name() == "vault.lock" {
                continue;
            }
            let bytes = fs::read(file.path()).unwrap();
            for marker in [
                filename.as_bytes(),
                content.as_slice(),
                b"DistinctSecretFolderName",
                b"long test passphrase",
            ] {
                assert!(!bytes.windows(marker.len()).any(|window| window == marker));
            }
        }
    }
    drop(vault);
    let vault = Vault::open(directory.path(), "long test passphrase").unwrap();
    assert!(vault.entry(&file.id).unwrap().starred);
    assert!(vault.is_pinned(&file.id));
    assert_eq!(contents(&vault, &file.id), content);
}

#[test]
fn corruption_and_chunk_substitution_never_return_unauthenticated_data() {
    let (_directory, mut vault) = fixture();
    let file = vault
        .import_file(None, "file.dat", Cursor::new(vec![7; CHUNK_SIZE * 2]))
        .unwrap();
    let manifest = vault.manifest(&file.id).unwrap();
    let first = vault.store.object(&manifest.chunks[0].id).unwrap();
    let second_path = vault
        .root
        .join("objects")
        .join(format!("{}.obj", manifest.chunks[1].id));
    let mut bytes = vec![1];
    bytes.extend_from_slice(&first.data);
    fs::write(&second_path, bytes).unwrap();
    assert!(matches!(
        vault.read(&file.id, CHUNK_SIZE as u64, &mut [0_u8; 12]),
        Err(Error::Authentication)
    ));
    fs::write(&second_path, [1_u8; 10]).unwrap();
    assert!(vault
        .read(&file.id, CHUNK_SIZE as u64, &mut [0_u8; 12])
        .is_err());
}

#[test]
fn replicated_metadata_can_be_opened_without_file_chunks() {
    let (_a_dir, mut a) = fixture();
    let file = a
        .import_file(None, "remote.txt", Cursor::new(b"download on demand"))
        .unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let mut b = Vault::attach_with_key(b_dir.path(), a.header(), a.key_bytes()).unwrap();
    let mut metadata: Vec<_> = a
        .export_objects()
        .unwrap()
        .into_iter()
        .filter(|object| object.kind != "chunk")
        .collect();
    metadata.reverse();
    for object in metadata {
        b.ingest_objects(vec![object]).unwrap();
    }
    assert_eq!(b.entry(&file.id).unwrap().name, "remote.txt");
    assert!(matches!(
        b.read(&file.id, 0, &mut [0_u8; 10]),
        Err(Error::OfflineUnavailable(_))
    ));
    for id in b.required_objects(&file.id).unwrap() {
        if !b.has_object(&id) {
            b.ingest_objects(vec![a.object(&id).unwrap()]).unwrap();
        }
    }
    assert_eq!(contents(&b, &file.id), b"download on demand");
}

#[test]
fn three_devices_converge_preserve_edits_and_delete_conflicts_in_all_orders() {
    let (_base_dir, mut base) = fixture();
    let file = base
        .import_file(None, "shared.txt", Cursor::new(b"base"))
        .unwrap();
    let (_a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    let (_c_dir, mut c) = replica(&base);
    a.write(&file.id, 0, b"AAAA").unwrap();
    b.write(&file.id, 0, b"BBBB").unwrap();
    c.set_trashed(&file.id, true).unwrap();
    let objects = [
        a.export_objects().unwrap(),
        b.export_objects().unwrap(),
        c.export_objects().unwrap(),
    ];
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let mut expected = None;
    for order in orders {
        let (_directory, mut replica) = replica(&base);
        for index in order {
            for object in &objects[index] {
                replica.ingest_objects(vec![object.clone()]).unwrap();
            }
        }
        let state = snapshot(&replica);
        if let Some(expected) = &expected {
            assert_eq!(&state, expected);
        } else {
            expected = Some(state);
        }
        let live_files: Vec<_> = replica
            .all_entries()
            .into_iter()
            .filter(|entry| !entry.trashed && entry.kind == EntryKind::File)
            .collect();
        assert_eq!(live_files.len(), 2);
        let bytes: BTreeSet<_> = live_files
            .iter()
            .map(|entry| contents(&replica, &entry.id))
            .collect();
        assert_eq!(bytes, BTreeSet::from([b"AAAA".to_vec(), b"BBBB".to_vec()]));
    }
}

#[test]
fn concurrent_moves_rename_and_folder_cycles_converge() {
    let (_base_dir, mut base) = fixture();
    let first = base.create_folder(None, "First").unwrap();
    let second = base.create_folder(None, "Second").unwrap();
    let file = base
        .import_file(None, "report.txt", Cursor::new(b"report"))
        .unwrap();
    let (_a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    a.move_entry(&first.id, Some(&second.id)).unwrap();
    b.move_entry(&second.id, Some(&first.id)).unwrap();
    a.rename(&file.id, "alpha.txt").unwrap();
    b.rename(&file.id, "beta.txt").unwrap();
    let a_objects = a.export_objects().unwrap();
    let b_objects = b.export_objects().unwrap();
    a.ingest_objects(b_objects).unwrap();
    b.ingest_objects(a_objects).unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    assert_eq!(
        a.all_entries()
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .count(),
        2
    );
    for entry in a.all_entries() {
        let mut visited = BTreeSet::new();
        let mut current = Some(entry.id);
        while let Some(id) = current {
            assert!(visited.insert(id.clone()));
            current = a.entry(&id).unwrap().parent_id;
        }
    }
}

#[test]
fn pending_and_pinned_chunks_are_not_evicted() {
    let (_directory, mut vault) = fixture();
    let file = vault
        .import_file(None, "pinned.dat", Cursor::new(vec![1_u8; CHUNK_SIZE * 2]))
        .unwrap();
    let pending = vault.pending_object_ids();
    // A transport may acknowledge chunks before publishing the dependent commit.
    let chunks: Vec<_> = pending
        .iter()
        .filter(|id| vault.store.kinds[*id] == "chunk")
        .cloned()
        .collect();
    vault.acknowledge_objects(&chunks).unwrap();
    vault.set_cache_limit_bytes(CHUNK_SIZE as u64).unwrap();
    assert_eq!(vault.evict_cache().unwrap(), 0);
    vault.set_pinned(&file.id, true).unwrap();
    vault.acknowledge_objects(&pending).unwrap();
    assert_eq!(vault.evict_cache().unwrap(), 0);
    vault.set_pinned(&file.id, false).unwrap();
    assert!(vault.evict_cache().unwrap() > 0);
    assert!(matches!(
        vault.read(&file.id, 0, &mut [0_u8; 1]),
        Err(Error::OfflineUnavailable(_))
    ));
}

#[test]
fn windows_names_and_destinations_are_validated() {
    let (_directory, mut vault) = fixture();
    for name in ["", "CON", "a/b", "a\\b", "x:", "AUX.txt", "ends.", "ends "] {
        assert!(vault.create_folder(None, name).is_err(), "{name}");
    }
    let folder = vault.create_folder(None, "Résumé").unwrap();
    assert!(matches!(
        vault.create_folder(None, "résumé"),
        Err(Error::AlreadyExists)
    ));
    assert!(vault.move_entry(&folder.id, Some(&folder.id)).is_err());
}

#[test]
fn acknowledged_history_removal_is_detected_on_reopen() {
    let (directory, mut vault) = fixture();
    vault.create_folder(None, "Folder").unwrap();
    let pending = vault.pending_object_ids();
    vault.acknowledge_objects(&pending).unwrap();
    let commit = vault
        .object_ids()
        .into_iter()
        .find(|(_, kind)| kind == "commit")
        .unwrap()
        .0;
    drop(vault);
    fs::remove_file(
        directory
            .path()
            .join("objects")
            .join(format!("{commit}.obj")),
    )
    .unwrap();
    assert!(matches!(
        Vault::open(directory.path(), "long test passphrase"),
        Err(Error::RollbackDetected)
    ));
}

#[test]
fn second_process_cannot_open_a_live_vault() {
    let (directory, _vault) = fixture();
    assert!(matches!(
        Vault::open(directory.path(), "long test passphrase"),
        Err(Error::InUse)
    ));
}

#[test]
fn trash_and_restore_preserve_folder_subtree_and_concurrent_child_edits() {
    let (_directory, mut vault) = fixture();
    let folder = vault.create_folder(None, "Folder").unwrap();
    let nested = vault.create_folder(Some(&folder.id), "Nested").unwrap();
    let file = vault
        .import_file(Some(&nested.id), "doc.txt", Cursor::new(b"base"))
        .unwrap();
    let (_other_dir, mut other) = replica(&vault);
    vault.set_trashed(&folder.id, true).unwrap();
    assert!(vault.all_entries().iter().all(|entry| entry.trashed));
    vault.set_trashed(&folder.id, false).unwrap();
    assert!(vault.all_entries().iter().all(|entry| !entry.trashed));
    assert_eq!(vault.entry(&file.id).unwrap().parent_id, Some(nested.id));
    vault.set_trashed(&folder.id, true).unwrap();
    other.write(&file.id, 0, b"edit").unwrap();
    vault
        .ingest_objects(other.export_objects().unwrap())
        .unwrap();
    assert!(!vault.entry(&file.id).unwrap().trashed);
    assert_eq!(contents(&vault, &file.id), b"edit");
    assert_eq!(vault.entry(&file.id).unwrap().parent_id, None);
}

#[test]
fn independently_materialized_conflict_copies_keep_both_edits() {
    let (_base_dir, mut base) = fixture();
    let file = base
        .import_file(None, "doc.txt", Cursor::new(b"base"))
        .unwrap();
    let (_a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    a.write(&file.id, 0, b"aaaa").unwrap();
    b.write(&file.id, 0, b"bbbb").unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    b.ingest_objects(a.export_objects().unwrap()).unwrap();
    let conflict = a
        .all_entries()
        .into_iter()
        .find(|entry| entry.id != file.id)
        .unwrap();
    a.write(&conflict.id, 0, b"1111").unwrap();
    b.write(&conflict.id, 0, b"2222").unwrap();
    let a_objects = a.export_objects().unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    b.ingest_objects(a_objects).unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    let content: BTreeSet<_> = a
        .all_entries()
        .iter()
        .filter(|entry| !entry.trashed)
        .map(|entry| contents(&a, &entry.id))
        .collect();
    assert!(content.contains(b"1111".as_slice()));
    assert!(content.contains(b"2222".as_slice()));
}

#[test]
fn exposed_conflict_survives_later_primary_write_rename_move_sync_and_restart() {
    let (_base_dir, mut base) = fixture();
    let folder = base.create_folder(None, "Destination").unwrap();
    let file = base
        .import_file(None, "doc.txt", Cursor::new(b"base"))
        .unwrap();
    let (a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    a.write(&file.id, 0, b"aaaa").unwrap();
    b.write(&file.id, 0, b"bbbb").unwrap();
    let a_objects = a.export_objects().unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    b.ingest_objects(a_objects).unwrap();
    let alias = a
        .all_entries()
        .into_iter()
        .find(|entry| entry.kind == EntryKind::File && entry.id != file.id)
        .unwrap();
    let alias_content = contents(&a, &alias.id);
    a.write(&file.id, 0, b"cccc").unwrap();
    a.rename(&file.id, "changed.txt").unwrap();
    a.move_entry(&file.id, Some(&folder.id)).unwrap();
    b.ingest_objects(a.export_objects().unwrap()).unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    for vault in [&a, &b] {
        assert_eq!(contents(vault, &alias.id), alias_content);
        assert_eq!(contents(vault, &file.id), b"cccc");
    }
    drop(a);
    let mut a = Vault::from_key(a_dir.path(), b.key_bytes()).unwrap();
    assert_eq!(contents(&a, &alias.id), alias_content);
    a.set_trashed(&alias.id, true).unwrap();
    b.ingest_objects(a.export_objects().unwrap()).unwrap();
    assert!(b.entry(&alias.id).unwrap().trashed);
}

#[test]
fn exposed_conflict_survives_a_remote_successor_that_never_observed_the_conflict() {
    let (_base_dir, mut base) = fixture();
    let file = base
        .import_file(None, "doc.txt", Cursor::new(b"base"))
        .unwrap();
    let (_a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    a.write(&file.id, 0, b"aaaa").unwrap();
    b.write(&file.id, 0, b"bbbb").unwrap();
    let (_observer_dir, mut observer) = replica(&a);
    observer
        .ingest_objects(b.export_objects().unwrap())
        .unwrap();
    let alias = observer
        .all_entries()
        .into_iter()
        .find(|entry| entry.id != file.id)
        .unwrap();
    let old_content = contents(&observer, &alias.id);
    // Both independent branches advance before learning about each other.
    a.write(&file.id, 0, b"1111").unwrap();
    b.write(&file.id, 0, b"2222").unwrap();
    observer
        .ingest_objects(a.export_objects().unwrap())
        .unwrap();
    observer
        .ingest_objects(b.export_objects().unwrap())
        .unwrap();
    assert_eq!(contents(&observer, &alias.id), old_content);
    a.ingest_objects(observer.export_objects().unwrap())
        .unwrap();
    b.ingest_objects(a.export_objects().unwrap()).unwrap();
    observer
        .ingest_objects(b.export_objects().unwrap())
        .unwrap();
    a.ingest_objects(observer.export_objects().unwrap())
        .unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    assert_eq!(snapshot(&a), snapshot(&observer));
}

#[test]
fn generated_conflicts_and_collisions_fit_windows_utf16_name_limit() {
    let (_base_dir, base) = fixture();
    let (_a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    let name = "😀".repeat(120);
    let first = a.import_file(None, &name, Cursor::new(b"aaaa")).unwrap();
    let second = b.import_file(None, &name, Cursor::new(b"bbbb")).unwrap();
    assert_ne!(first.id, second.id);
    let a_objects = a.export_objects().unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    b.ingest_objects(a_objects).unwrap();
    a.write(&first.id, 0, b"1111").unwrap();
    b.write(&first.id, 0, b"2222").unwrap();
    let a_objects = a.export_objects().unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    b.ingest_objects(a_objects).unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    let mut names = BTreeSet::new();
    for entry in a.all_entries() {
        validate_name(&entry.name).unwrap();
        assert!(entry.name.encode_utf16().count() <= 240);
        assert!(names.insert(entry.name.to_lowercase()));
    }
}

#[test]
fn adopt_header_authenticates_the_whole_rewrap_and_updates_memory_and_disk() {
    let (_base_dir, base) = fixture();
    let (a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    b.change_password("long test passphrase", "new password from another PC")
        .unwrap();
    let updated = b.header().clone();
    let original_disk = fs::read(a_dir.path().join(HEADER_FILENAME)).unwrap();
    let original_memory = serde_json::to_vec(a.header()).unwrap();
    a.verify_header(&updated).unwrap();
    let mut invalid = updated.clone();
    invalid.wrapped_key = "00".repeat(72);
    assert!(a.verify_header(&invalid).is_err());
    assert_eq!(
        fs::read(a_dir.path().join(HEADER_FILENAME)).unwrap(),
        original_disk
    );
    assert_eq!(serde_json::to_vec(a.header()).unwrap(), original_memory);
    a.adopt_header(&updated).unwrap();
    assert!(a
        .change_password("long test passphrase", "obsolete must fail")
        .is_err());
    let persisted = fs::read(a_dir.path().join(HEADER_FILENAME)).unwrap();
    let mut tampered = updated.clone();
    tampered.wrapped_key.replace_range(..2, "ff");
    assert!(a.adopt_header(&tampered).is_err());
    tampered = updated.clone();
    tampered.salt = hex::encode(random::<16>());
    assert!(a.adopt_header(&tampered).is_err());
    let (_other_dir, other) = fixture();
    assert!(a.adopt_header(other.header()).is_err());
    assert_eq!(
        fs::read(a_dir.path().join(HEADER_FILENAME)).unwrap(),
        persisted
    );
    drop(a);
    assert!(Vault::open(a_dir.path(), "long test passphrase").is_err());
    assert!(Vault::open(a_dir.path(), "new password from another PC").is_ok());
}

#[test]
fn preparing_a_password_change_has_no_disk_or_memory_side_effects() {
    let (directory, mut vault) = fixture();
    let before = fs::read(directory.path().join(HEADER_FILENAME)).unwrap();
    let prepared = vault
        .prepare_password_change("long test passphrase", "prepared new password")
        .unwrap();
    assert_eq!(
        fs::read(directory.path().join(HEADER_FILENAME)).unwrap(),
        before
    );
    assert_eq!(
        serde_json::to_vec(vault.header()).unwrap(),
        serde_json::to_vec(&Vault::read_header(directory.path()).unwrap()).unwrap()
    );
    assert!(vault.header().unwrap("long test passphrase").is_ok());
    assert!(vault.header().unwrap("prepared new password").is_err());
    assert!(prepared.unwrap("prepared new password").is_ok());
    vault.adopt_header(&prepared).unwrap();
    assert!(vault.header().unwrap("prepared new password").is_ok());
}

#[test]
fn atomic_relocate_replaces_a_document_with_one_commit_and_retains_target_history() {
    let (directory, mut vault) = fixture();
    let folder = vault.create_folder(None, "Documents").unwrap();
    let document = vault
        .import_file(Some(&folder.id), "doc.txt", Cursor::new(b"old document"))
        .unwrap();
    let temporary = vault
        .import_file(None, "save.tmp", Cursor::new(b"new document"))
        .unwrap();
    let (_replica_dir, mut replica) = replica(&vault);
    let before = vault.commits.len();
    vault
        .relocate(&temporary.id, Some(&folder.id), "doc.txt", true)
        .unwrap();
    assert_eq!(vault.commits.len(), before + 1);
    assert_eq!(vault.entry(&temporary.id).unwrap().name, "doc.txt");
    assert_eq!(
        vault.entry(&temporary.id).unwrap().parent_id,
        Some(folder.id.clone())
    );
    assert_eq!(contents(&vault, &temporary.id), b"new document");
    assert!(vault.entry(&document.id).unwrap().trashed);
    assert_eq!(contents(&vault, &document.id), b"old document");
    let objects = vault.export_objects().unwrap();
    let commit = objects
        .into_iter()
        .find(|object| object.kind == "commit" && !replica.has_object(&object.id))
        .unwrap();
    replica.ingest_objects(vec![commit]).unwrap();
    assert_eq!(snapshot(&vault), snapshot(&replica));
    assert_eq!(replica.list(Some(&folder.id), false).len(), 1);
    assert!(replica
        .list(None, false)
        .iter()
        .all(|entry| entry.id != temporary.id));
    drop(vault);
    let vault = Vault::open(directory.path(), "long test passphrase").unwrap();
    assert!(vault.entry(&document.id).unwrap().trashed);
    assert_eq!(contents(&vault, &temporary.id), b"new document");
}

#[test]
fn failed_atomic_relocate_leaves_source_and_target_untouched() {
    let (_directory, mut vault) = fixture();
    let folder = vault.create_folder(None, "Folder").unwrap();
    let nested = vault.create_folder(Some(&folder.id), "Nested").unwrap();
    let target = vault
        .import_file(None, "doc.txt", Cursor::new(b"original"))
        .unwrap();
    let source = vault
        .import_file(None, "temp.tmp", Cursor::new(b"replacement"))
        .unwrap();
    let before = snapshot(&vault);
    let before_commits = vault.commits.len();
    assert!(matches!(
        vault.relocate(&source.id, None, "DOC.TXT", false),
        Err(Error::AlreadyExists)
    ));
    assert!(vault.relocate(&source.id, None, "Folder", true).is_err());
    assert!(vault.relocate(&folder.id, None, "doc.txt", true).is_err());
    assert!(vault
        .relocate(&folder.id, Some(&nested.id), "Moved", false)
        .is_err());
    assert_eq!(snapshot(&vault), before);
    assert_eq!(vault.commits.len(), before_commits);
    assert_eq!(contents(&vault, &target.id), b"original");
    assert_eq!(contents(&vault, &source.id), b"replacement");
}

#[test]
fn atomic_relocation_and_concurrent_target_edit_converge_without_losing_either_content() {
    let (_base_dir, mut base) = fixture();
    let document = base
        .import_file(None, "doc.txt", Cursor::new(b"original"))
        .unwrap();
    let temporary = base
        .import_file(None, "save.tmp", Cursor::new(b"new text"))
        .unwrap();
    let (_a_dir, mut a) = replica(&base);
    let (_b_dir, mut b) = replica(&base);
    a.relocate(&temporary.id, None, "doc.txt", true).unwrap();
    b.write(&document.id, 0, b"changed!").unwrap();
    let a_objects = a.export_objects().unwrap();
    a.ingest_objects(b.export_objects().unwrap()).unwrap();
    b.ingest_objects(a_objects).unwrap();
    assert_eq!(snapshot(&a), snapshot(&b));
    assert!(!a.entry(&document.id).unwrap().trashed);
    assert_eq!(contents(&a, &document.id), b"changed!");
    assert_eq!(contents(&a, &temporary.id), b"new text");
    assert_ne!(
        a.entry(&document.id).unwrap().name.to_lowercase(),
        a.entry(&temporary.id).unwrap().name.to_lowercase()
    );
}

#[test]
fn authenticated_but_invalid_commits_are_quarantined_without_blocking_the_vault() {
    let (_a_dir, mut a) = fixture();
    let file = a
        .import_file(None, "kept.txt", Cursor::new(b"still readable"))
        .unwrap();
    let (_forger_dir, mut forger) = replica(&a);
    let (b_dir, mut b) = replica(&a);
    let invalid_id = new_id();
    let invalid = Commit {
        id: invalid_id.clone(),
        parents: forger.heads.clone(),
        device_id: forger.local.device_id.clone(),
        modified: 1,
        operation: Operation::Relocate {
            id: file.id.clone(),
            parent: None,
            name: "never.txt".into(),
            displaced: Some(new_id()),
        },
    };
    forger
        .store
        .put_metadata("commit", &invalid_id, &invalid)
        .unwrap();
    let descendant_id = new_id();
    let descendant = Commit {
        id: descendant_id.clone(),
        parents: vec![invalid_id.clone()],
        device_id: forger.local.device_id.clone(),
        modified: 2,
        operation: Operation::Rename {
            id: file.id.clone(),
            name: "descendant.txt".into(),
        },
    };
    forger
        .store
        .put_metadata("commit", &descendant_id, &descendant)
        .unwrap();
    b.ingest_objects(vec![
        forger.object(&invalid_id).unwrap(),
        forger.object(&descendant_id).unwrap(),
    ])
    .unwrap();
    assert_eq!(b.entry(&file.id).unwrap().name, "kept.txt");
    assert_eq!(contents(&b, &file.id), b"still readable");
    drop(b);
    let reopened = Vault::open(b_dir.path(), "long test passphrase").unwrap();
    assert_eq!(reopened.entry(&file.id).unwrap().name, "kept.txt");
}

#[test]
fn tampered_chunks_are_rejected_before_they_are_stored() {
    let (_a_dir, mut a) = fixture();
    let file = a
        .import_file(None, "remote.bin", Cursor::new(b"chunk payload"))
        .unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let mut b = Vault::attach_with_key(b_dir.path(), a.header(), a.key_bytes()).unwrap();
    let metadata: Vec<_> = a
        .export_objects()
        .unwrap()
        .into_iter()
        .filter(|object| object.kind != "chunk")
        .collect();
    b.ingest_objects(metadata).unwrap();
    let chunk_id = b
        .required_objects(&file.id)
        .unwrap()
        .into_iter()
        .find(|id| !b.has_object(id))
        .unwrap();
    let genuine = a.object(&chunk_id).unwrap();
    let mut tampered = genuine.clone();
    let last = tampered.data.len() - 1;
    tampered.data[last] ^= 1;
    assert!(matches!(
        b.ingest_objects(vec![tampered]),
        Err(Error::Authentication)
    ));
    assert!(!b.has_object(&chunk_id));
    b.ingest_objects(vec![genuine]).unwrap();
    assert_eq!(contents(&b, &file.id), b"chunk payload");
}
