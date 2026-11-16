# Security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub's **Report a vulnerability** button on this repository's **Security** tab (private vulnerability reporting). Do not open public issues or pull requests for security problems, and do not include real account data, tokens, or vault contents in reports. Synthetic reproductions are preferred.

## Status

pCloud Vault uses a new, **unaudited** vault protocol built on established primitives: Argon2id, XChaCha20-Poly1305, and the operating system's secret storage (Windows DPAPI, the macOS Keychain, or a Linux Secret Service). It has not had an independent security review. Do not rely on it as the only copy of important data.

## Protection boundary

- Cloud storage receives only encrypted vault objects, but the provider can still observe object sizes, counts, and access timing.
- Files under **pCloud Files** are ordinary, unencrypted account files. So are backups and syncs to pCloud, uploads to pCloud, public links, shared folders, and files received through file requests (until the app moves them into the Vault, after which the plain copy sits in the pCloud trash).
- Backup and sync indexes of Vault jobs are encrypted at rest; the local folder paths of jobs and the app's recycle folder of files removed by a sync are not.
- The pCloud sign-in token (an OAuth token, or a full website session token when you use website sign-in) is protected per user and computer: with current-user DPAPI on Windows, and on macOS and Linux with XChaCha20-Poly1305 under a random device key kept in the login Keychain or the Secret Service keyring. Anyone who can run code as your user (with the keyring unlocked) can use it. Forgetting the computer logs a website session out at pCloud.
- An unlocked vault in a trusted user session is accessible to that user's processes. Exported or opened decrypted copies, malware, and operating-system paging are outside the protection boundary.
- A password change rewraps the vault key. It does not revoke previously trusted computers or older wrapped keys kept in cloud history.

See [docs/VAULT_FORMAT.md](docs/VAULT_FORMAT.md) for the exact format.
