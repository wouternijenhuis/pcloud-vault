# pCloud Vault

A standalone Windows 11 client for pCloud with a quiet, premium interface and local authenticated encryption. Browse and manage files inside the app. No filesystem driver, mounted drive, or separate encryption application is required.

## Getting started

Run the Windows installer or the portable `pcloud-vault.exe`. Connect your pCloud account, then create or unlock Private Vault with one unique passphrase of at least 12 characters. Remembering the PC protects its unlock key with your Windows account and starts the app quietly at sign-in. Closing the window keeps transfers running in the tray; use the tray menu to open, lock, or quit.

**Connecting pCloud:** choose **Sign in on pCloud website**. A separate private window shows pCloud's own sign-in page, including two-step verification, and no developer registration is needed. Alternatively, register your own personal-use OAuth application in pCloud's developer portal (pCloud may require manual approval) and use its public client ID with callback `http://127.0.0.1:43187/callback`. No client secret is embedded. You can also choose **Work locally** to create a real encrypted vault and import/export files on this PC only. See [pCloud setup](docs/PCLOUD_SETUP.md).

Private Vault is separate from **pCloud Files**, which is explicitly unencrypted. Importing copies files and preserves the originals. Folder drag-and-drop imports are supported. Favorites, offline pins, encrypted caching, search, image previews, trash/restore, password changes, and recovery-key export are available inside the app. Ordinary cloud folders/files support browsing, rename, move, download, and trash; individual cloud files can be copied into Private Vault. Restore ordinary cloud deletions through pCloud's own Trash.

Exporting or choosing **Export and open** creates a decrypted copy at a location you choose. Exports never overwrite existing files. These explicit copies and the original files you imported remain outside the encrypted vault. The client does not leave decrypted working files in its managed cache.

## Encryption and sync

- RustCrypto Argon2id (256 MiB, three passes, one lane) protects a random 256-bit root key. XChaCha20-Poly1305 authenticates encrypted 4 MiB file chunks and metadata.
- Filenames, folders, previews managed on disk, and local preferences stay encrypted. pCloud sign-in tokens and remembered keys use current-user Windows DPAPI. Passwords are never persisted.
- Immutable encrypted commits and causal merge preserve concurrent edits as recoverable conflict copies. Uploads verify object bytes before publishing dependent commits. An encrypted local journal preserves queued work across restarts.
- Pin folders or files to keep their contents offline on this PC. The default 10 GiB cache limit is soft when pinned or pending data needs more space. Favorites and pins are per-PC preferences.
- Each new PC connects the same account and unlocks the vault with its password once. A password change rewraps the root key; it does not revoke previously trusted PCs or older password wrappers held in cloud history.

This is an initial implementation with a new vault protocol, **not an independently audited security product**. Keep independent copies of important files while validating it. Cloud providers still see object sizes and access timing; an unlocked Windows session, malware, operating-system paging, and external applications are outside the protection boundary. No automatic remote garbage collection is performed. See the exact [vault format and limitations](docs/VAULT_FORMAT.md).

## Recovery without the interface

The separate `vault-recovery.exe` verifies or exports a complete local/downloaded vault. Quit the desktop app before opening its local vault with the recovery tool, since each local replica has an exclusive process lock.

```powershell
.\vault-recovery.exe verify 'D:\EncryptedVault'
.\vault-recovery.exe export 'D:\EncryptedVault' 'D:\NewRecoveredFolder'
# Use the offline recovery key instead of the vault password:
.\vault-recovery.exe verify 'D:\EncryptedVault' --recovery
```

Secrets are prompted interactively, never supplied as command-line arguments. For automation, `--password-stdin` explicitly reads one secret line from a pipe. Do not put real secrets into shell history.

Cloud layout is `.pcloud-vault/vault.json` plus `objects/c-<id>.obj`, `m-<id>.obj`, and `h-<id>.obj`. pCloud's own website and apps hide this dot-folder, so files you add to Private Vault never appear in pCloud under their real names; see them in this app. The prefixes distinguish ciphertext chunks, manifests, and history without revealing original names. Download the whole `.pcloud-vault` directory before offline recovery, then normalize its layout:

```powershell
.\vault-recovery.exe hydrate 'D:\DownloadedCloudVault' 'D:\NewEncryptedReplica'
.\vault-recovery.exe verify 'D:\NewEncryptedReplica'
.\vault-recovery.exe export 'D:\NewEncryptedReplica' 'D:\NewPlaintextExport'
```

Hydration copies encrypted objects; verification checks their authentication before export. Missing cloud history or ciphertext cannot be recovered from a password alone.

## Building from source

Developer prerequisites are Rust stable with MSVC build tools, Node.js, and pnpm. Users of the packaged app do not need them. All UI assets are bundled, and no remote JavaScript, fonts, analytics, or application server is used.

```powershell
. .\scripts\dev-env.ps1 # optional scoped-toolchain / installed Visual Studio discovery
pnpm install --frozen-lockfile
pnpm --dir frontend install --frozen-lockfile
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm --dir frontend test
pnpm --dir frontend build
cargo build --release -p vault-cli
pnpm build
```

`pnpm preview` opens a clearly labeled **browser demo** using sample data; it never performs real cloud operations. `pnpm dev` starts the native Tauri app. For isolated manual QA, launch the native executable with `--data-dir <new-test-directory>`; this disables Windows startup registration for that run. Standard data is stored under the current user's application-data directory for `dev.pcloudvault.desktop`.

Source license: GPL-3.0-only. No GitHub Actions workflows, deployment, publishing, or automatic updater are configured.

## Contributing

This is a public repository. Read [CLAUDE.md](CLAUDE.md) (identical rules in [AGENTS.md](AGENTS.md)) before contributing, and enable the public-safety hooks once per clone:

```powershell
git config core.hooksPath .githooks
```

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md), never in public issues.
