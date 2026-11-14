# Working in this repository

## This is a public repository

Everything committed, pushed, or written in issues, pull requests, reviews, and commit messages is world-readable and permanently archived. Treat every change as a publication.

Never commit or paste:

- Personal names, email addresses, phone numbers, or account identifiers. Commits use the GitHub username with its `users.noreply.github.com` address.
- Local machine paths, Windows/Unix usernames, hostnames, or drive layouts from a developer's machine. Use placeholders such as `D:\EncryptedVault` or `<data-dir>`.
- Tokens, passwords, OAuth client IDs or secrets, recovery keys, API responses containing account data, `.env*` files, or DPAPI blobs.
- Real pCloud account data, real file names or contents from anyone's vault, logs, crash dumps, or screenshots that show any of the above.

Rules:

- Test fixtures and sample UI data are synthetic. Use `example.com`, `.invalid`, or `.test` domains.
- Run `pwsh -NoProfile -File scripts/check-public.ps1` before committing and pushing. Enable the hooks once per clone with `git config core.hooksPath .githooks`. Put personal terms the scan should also block (your real name, machine username) one per line in the untracked file `.git/info/public-denylist`, never in a tracked file.
- Fix a failing check by removing the data. Do not weaken the patterns, and do not bypass the hooks with `--no-verify`.
- If something sensitive was ever pushed, rewriting history is not enough. Rotate or revoke the secret first, then report it to the maintainer.
- Security vulnerabilities go through private reporting ([SECURITY.md](SECURITY.md)), never public issues or PR descriptions.
- New dependencies need a stated reason, an actively maintained source, and a license compatible with GPL-3.0-only. Do not add telemetry, analytics, remote fonts/scripts, or auto-updaters.
- GitHub Actions, if ever added:
  - minimal `permissions:`
  - no `pull_request_target` that checks out untrusted code
  - never echo secrets
  - third-party actions pinned to a full commit SHA
  - no publishing from pull requests
- Never claim audited security. The vault protocol is new and unaudited. Keep the protection boundary in `docs/VAULT_FORMAT.md` accurate.

## Project notes

- Rust workspace: `crates/vault-core` (crypto, store, sync), `crates/pcloud-api` (transport, OAuth, DPAPI), `crates/vault-cli` (recovery), `src-tauri` (desktop shell). React/TypeScript UI is in `frontend`.
- Cloud credentials and file bytes stay in Rust. The UI talks only through the restricted Tauri commands in `src-tauri/capabilities/main.json`.
- Validation:
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `pnpm --dir frontend test`
  - `pnpm --dir frontend build`
  - `pnpm build` (NSIS installer)
- Toolchain helper: `. .\scripts\dev-env.ps1`. Set `PCLOUD_VAULT_TOOLCHAINS` to a portable toolchain folder if Rust/MSVC are not installed system-wide.
