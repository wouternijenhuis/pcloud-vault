# Implementation and acceptance

The final user-selected scope is a standalone Windows client. The initial mounted-drive requirement was replaced with in-app file management to avoid extra filesystem components. The delivered app has no WinFsp dependency, driver installer, or drive-mount control. The Rust core still supports random access and atomic relocation for future adapters, but these are not runtime prerequisites.

## Boundaries

- `vault-core`: encryption, authenticated headers and chunks, file operations, durable object/journal storage, per-PC pins, deterministic causal merge and conflict preservation.
- `pcloud-api`: regional HTTPS API transport, browser OAuth callback, immutable upload/read-back verification and current-user DPAPI.
- `src-tauri`: restricted local IPC, secret persistence, transfer orchestration, import/export dialogs, encrypted cache hydration, tray lifecycle and startup.
- `frontend`: bundled React/TypeScript interface. Browser-only sample mode is labeled; native operations use typed IPC and surface backend errors.
- `vault-cli`: independent verification, decrypted export and cloud-layout hydration.

The frontend cannot directly call the network, launch shell commands, or read arbitrary files. Rust performs cloud requests and native file operations. Native drag/drop paths and explicit file-dialog choices are imported through the encryption core. The UI only receives file metadata and bounded raster image previews while unlocked; it clears private views on lock.

## Persistence and publication

Content objects and encrypted commits are durable before a local mutation succeeds. Pending dependencies are uploaded in chunk/manifest/causal-commit order and acknowledged only after read-back verification. Remote root identity is authenticated before exchanging objects, including after an empty-account bootstrap race. Ciphertext metadata is reconciled without downloading the entire account's content. Files and pinned folder descendants hydrate on demand; pending/pinned chunks are protected from eviction. Local objects and checkpoints are published with write-through renames.

Incoming chunks are authenticated against a known manifest before they are stored, because immutable storage never replaces an object. Commits that authenticate but are semantically invalid (for example from a buggy or incompatible client) are quarantined deterministically together with their descendants. This keeps the vault readable on every PC. Remote objects that fail validation are skipped and retried on the next sync instead of pausing replication.

Reconciliation lists the remote objects folder on every sync. The pCloud diff cursor is recorded, and a rejected or expired cursor falls back to a fresh position, but incremental diff-driven discovery is not implemented yet.

Password changes first prepare a complete authenticated wrapper, persist `pending-header.json`, and adopt it locally. Sync verifies remote identity without reverting the pending local password, publishes/read-back verifies the new wrapper, then removes the pending marker. A restart replays a durable pending wrapper before unlock, and only if it belongs to the same vault. Each PC records the random salts of the wrappers it has superseded and never adopts them again, so a reverted cloud header cannot silently restore an older password. A wrapper that was changed concurrently on another PC wins over an unpublished local change, and the app reports this. Password reset/revocation across previously trusted devices is deliberately not implied.

Imports reject the managed-storage directory, its descendants, and source ancestors that contain it, avoiding self-ingestion. Exports use exclusive file creation and reject destinations in managed storage. File dialogs do not retain vault locks. Recovery-key extraction is deferred until after its save dialog and requires the vault to still be unlocked.

## Validation status

Local checks and native smoke-test results are recorded in the pull request. Tests include published XChaCha test vectors, bounded malicious headers, tampering/substitution, rewrap/recovery, random writes/truncation, immutable persistence, plaintext-marker disk scans, pending/pinned eviction protection, three-device convergence in delivery permutations, durable conflict copies, Unicode collision names, folder trash/restore, atomic replacement, DPAPI, OAuth callback state/host/replay validation, API upload flags, standalone import/export, and recovery CLI subprocess execution.

Live pCloud account authorization, existing-file scope, real remote replication and multi-PC network tests remain pending the provider's manual OAuth app approval. Mock transport tests and in-memory replication tests do not establish those outcomes. This new protocol still needs an independent security audit before audited-security claims or sole-copy use.

No hosted CI workflows, deployment, or release automation are configured.
