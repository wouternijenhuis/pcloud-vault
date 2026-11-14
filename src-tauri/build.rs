fn main() {
    const COMMANDS: &[&str] = &[
        "status",
        "list_entries",
        "create_vault",
        "unlock_vault",
        "lock_vault",
        "forget_device",
        "connect_pcloud",
        "save_settings",
        "new_folder",
        "rename_entry",
        "move_entry",
        "trash_entry",
        "restore_entry",
        "toggle_favorite",
        "toggle_pin",
        "import_files",
        "import_paths",
        "import_cloud_file",
        "download_entry",
        "open_entry",
        "preview_entry",
        "sync_now",
        "change_password",
        "export_recovery",
        "get_activity",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build desktop resources");
}
