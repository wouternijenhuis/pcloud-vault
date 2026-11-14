# Changelog

All notable changes to pCloud Vault are listed here. The project follows [Semantic Versioning](https://semver.org/). A release is published automatically when a merge to `main` carries a new version in `src-tauri/tauri.conf.json`.

## [0.1.0]

First public release for Windows 11 x64.

- **Private Vault:** files are encrypted on your PC (Argon2id and XChaCha20-Poly1305 in 4 MiB authenticated chunks) before anything is uploaded. Filenames and folders are encrypted too.
- **pCloud Files:** browse, rename, move, download and trash your regular pCloud files. They are clearly labelled as not encrypted.
- **Sign in on pCloud's website:** use email, Google, Apple or Facebook in a separate private window. No developer app approval is needed. OAuth with your own registered app is also supported.
- **Several PCs:** the vault syncs through pCloud. Concurrent edits are kept as conflict copies, and changes are discovered through pCloud's change feed.
- **Offline access:** keep files offline, with a configurable encrypted cache (10 GiB by default).
- **Welcome guide** on first start, right-click menus, search, previews, favorites, trash and restore, light and dark themes.
- **Recovery:** an offline recovery key and the separate `vault-recovery.exe`, which verifies and exports a vault without the app.
- **Remembered PCs:** this PC's unlock key and the pCloud sign-in are protected with Windows DPAPI. Your password is never stored.
