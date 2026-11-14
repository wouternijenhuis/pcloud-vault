# Security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub's **Report a vulnerability** button on this repository's **Security** tab (private vulnerability reporting). Do not open public issues or pull requests for security problems, and do not include real account data, tokens, or vault contents in reports. Synthetic reproductions are preferred.

## Status

pCloud Vault uses a new, **unaudited** vault protocol built on established primitives: Argon2id, XChaCha20-Poly1305, and Windows DPAPI. It has not had an independent security review. Do not rely on it as the only copy of important data.

## Protection boundary

- Cloud storage receives only encrypted vault objects, but the provider can still observe object sizes, counts, and access timing.
- Files under **pCloud Files** are ordinary, unencrypted account files.
- The pCloud sign-in token (an OAuth token, or a full website session token when you use website sign-in) is protected with current-user Windows DPAPI. Anyone who can run code as your Windows user can use it. Forgetting the PC logs a website session out at pCloud.
- An unlocked vault on a trusted Windows session is accessible to that user's processes. Exported or opened decrypted copies, malware, and operating-system paging are outside the protection boundary.
- A password change rewraps the vault key. It does not revoke previously trusted PCs or older wrapped keys kept in cloud history.

See [docs/VAULT_FORMAT.md](docs/VAULT_FORMAT.md) for the exact format.
