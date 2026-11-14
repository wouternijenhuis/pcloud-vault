use std::{
    io::Write,
    process::{Command, Stdio},
};
use vault_core::Vault;

#[test]
fn recovery_binary_verifies_exports_and_refuses_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let mut vault = Vault::create(&root, "integration-password-phrase").unwrap();
    let folder = vault.create_folder(None, "Family").unwrap();
    vault
        .import_file(
            Some(&folder.id),
            "note.txt",
            &b"Recover this private text"[..],
        )
        .unwrap();
    drop(vault);
    let output = temp.path().join("export");
    for operation in ["verify", "export"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vault-recovery"));
        command.arg(operation).arg(&root);
        if operation == "export" {
            command.arg(&output);
        }
        let mut child = command
            .arg("--password-stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"integration-password-phrase\n")
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert_eq!(
        std::fs::read(output.join("Family/note.txt")).unwrap(),
        b"Recover this private text"
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_vault-recovery"))
        .arg("export")
        .arg(&root)
        .arg(&output)
        .arg("--password-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"integration-password-phrase\n")
        .unwrap();
    assert!(!child.wait().unwrap().success());

    let cloud = temp.path().join("cloud");
    std::fs::create_dir_all(cloud.join("objects")).unwrap();
    std::fs::copy(root.join("vault.json"), cloud.join("vault.json")).unwrap();
    for object in std::fs::read_dir(root.join("objects")).unwrap() {
        let object = object.unwrap();
        let bytes = std::fs::read(object.path()).unwrap();
        let kind = match bytes[0] {
            1 => "c",
            2 => "m",
            3 => "h",
            _ => panic!("unknown fixture object kind"),
        };
        std::fs::write(
            cloud
                .join("objects")
                .join(format!("{kind}-{}", object.file_name().to_string_lossy())),
            bytes,
        )
        .unwrap();
    }
    let hydrated = temp.path().join("hydrated");
    let result = Command::new(env!("CARGO_BIN_EXE_vault-recovery"))
        .arg("hydrate")
        .arg(&cloud)
        .arg(&hydrated)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let opened = Vault::open(&hydrated, "integration-password-phrase").unwrap();
    assert_eq!(opened.all_entries().len(), 2);
    let note = opened
        .all_entries()
        .into_iter()
        .find(|entry| entry.name == "note.txt")
        .unwrap();
    let mut bytes = [0u8; 128];
    let length = opened.read(&note.id, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..length], b"Recover this private text");
}
