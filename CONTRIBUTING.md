# Contributing to pCloud Vault

Thanks for helping. This is a small, security-sensitive project, so changes favor clarity over cleverness.

## Before you start

- Read [CLAUDE.md](CLAUDE.md) (the same rules are in [AGENTS.md](AGENTS.md)). **This is a public repository:** never commit personal data, local paths, tokens, account details or real vault contents.
- Report security problems privately. See [SECURITY.md](SECURITY.md).
- For larger changes, open an issue first so we can agree on the approach.

## Development setup

Requirements: Rust stable, Node.js 22 or later and pnpm, plus per platform:

- **Windows 11 x64:** the MSVC build tools. WebView2 is included with Windows 11.
- **macOS 11+:** the Xcode command-line tools.
- **Linux:** the [Tauri system packages](https://v2.tauri.app/start/prerequisites/#linux), for example on Debian/Ubuntu `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libdbus-1-dev`, and a Secret Service keyring for saved sign-ins.

```powershell
git config core.hooksPath .githooks   # public-safety checks before commit and push
pnpm install --frozen-lockfile
pnpm --dir frontend install --frozen-lockfile
pnpm dev                              # run the desktop app
pnpm preview                          # browser demo with sample data (no cloud access)
```

## Checks

Run these before opening a pull request. CI runs the same checks on Windows, Linux and macOS for ready pull requests.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm --dir frontend test
pnpm --dir frontend build
```

## Pull requests

- Keep each pull request focused. Explain what changes for users and how you tested it.
- Add or update tests for behavior changes, especially anything touching encryption, sync or file handling.
- Use synthetic test data only (`example.com`, `.invalid`, `.test`).
- New dependencies need a reason and a license compatible with GPL-3.0-only.
- Commits use your GitHub `users.noreply.github.com` address.

## Releases

Maintainers bump `version` in `src-tauri/tauri.conf.json`, `package.json` and the workspace `Cargo.toml`, and add a `CHANGELOG.md` section. Merging to `main` then publishes the release automatically.

By contributing you agree that your contributions are licensed under the [GPL-3.0-only](LICENSE) license.
