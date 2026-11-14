# Changelog

All notable changes to pCloud Vault are listed here. The project follows [Semantic Versioning](https://semver.org/). A release is published automatically when a merge to `main` carries a new version in `src-tauri/tauri.conf.json`.

## [0.2.0]

pCloud Vault now runs on Linux and macOS as well as Windows.

- **Linux (x64):** an AppImage and a `.deb` package. Saved sign-ins and remembered unlocks use a device key kept in your Secret Service keyring (GNOME Keyring or KWallet).
- **macOS (Apple Silicon and Intel):** a universal `.dmg`. Saved secrets use a device key kept in your login Keychain. Clicking the Dock icon reopens the window.
- **Start in the background at sign-in** on every platform while a computer is remembered: Windows Run key, Linux XDG autostart entry, macOS LaunchAgent.
- **Recovery tool** `vault-recovery` is included for all three platforms.
- Windows keeps using DPAPI, so data from 0.1.0 keeps working without changes.
- Release downloads now include build provenance and one `SHA256SUMS.txt` for every file.

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
