# Changelog

All notable changes to pCloud Vault are listed here. The project follows [Semantic Versioning](https://semver.org/). A release is published automatically when a merge to `main` carries a new version in `src-tauri/tauri.conf.json`.

## [0.3.1]

A maintenance release: every library is on its latest version, with no change to how your files are stored.

- **Up-to-date foundations:** Tauri 2.12, React 19 and the newest Rust cryptography, networking and keyring libraries.
- **Security fixes from libraries:** includes the fixes for RUSTSEC-2026-0592 (`url`) and RUSTSEC-2026-0594 (`cookie`).
- **Secure connections use your system's certificate store** to verify pCloud's servers.
- **Linux:** the keyring connection uses a pure-Rust Secret Service client. If the keyring was not running when pCloud Vault started, start it and then restart the app to remember this computer.
- **Large vaults stay fast:** importing, backing up or syncing thousands of files no longer slows down with every file. Importing 2,000 files is about 3.5 times faster, and much larger vaults open in a fraction of a second.
- **Safer sharing:** links, shared folders and file requests refuse your whole pCloud (the top folder), since it holds the encrypted vault.
- **Recovery key file is private:** on macOS and Linux an exported recovery key can now only be read by your own user account.
- **Vault format guarded by tests:** new known-answer tests and a sample vault check that every future update keeps opening existing vaults.

## [0.3.0]

pCloud Vault now offers the sections of a full pCloud client, with encryption wherever you want it.

- **Choose your protection:** everything encrypted, choose each time, or only the Vault. Set it in the welcome guide or in Settings. Every destination, job, file and photo is labelled *Encrypted* or *Not encrypted*.
- **Upload** page with drag and drop, Browse files and Browse folder, a destination picker and a transfer list.
- **Back up** (one-way) and **Sync** (two-way) folders from this computer to the Vault or to pCloud. Changes on both sides are kept as conflict copies, large deletions pause for your review, and files a sync removes from this computer go to a recycle folder for 30 days. Indexes of Vault jobs are encrypted.
- **Photos** timeline across the Vault and pCloud, with in-memory thumbnails.
- **Shared:** public links and shared folders for regular pCloud files. Vault files are never shared.
- **File requests**, optionally moving received files into the Vault automatically.
- **Account** (storage use), **Help**, **About**, and a status bar with upload and download activity.
- Settings for theme (System, Light or Dark) and starting at sign-in.
- **pCloud Files** can now be renamed, moved and moved to pCloud's trash, and actions on several selected items apply to all of them.
- No new third-party libraries: folder scanning, recycling and thumbnails use what the app already had.

## [0.2.0]

pCloud Vault now runs on Linux and macOS as well as Windows.

- **Linux (x64):** an AppImage and a `.deb` package. Saved sign-ins and remembered unlocks use a device key kept in your Secret Service keyring (GNOME Keyring or KWallet).
- **macOS (Apple Silicon and Intel):** a universal `.dmg`. Saved secrets use a device key kept in your login Keychain. Clicking the Dock icon reopens the window.
- **Start in the background at sign-in** on every platform while a computer is remembered: Windows Run key, Linux XDG autostart entry, macOS LaunchAgent.
- Without a keyring (for example a minimal Linux desktop), the app still works for the session and explains in Activity why it cannot remember the computer or the sign-in.
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
