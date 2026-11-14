# Connecting pCloud

The desktop app needs a registered pCloud OAuth application's **public client ID**. It does not need or embed a client secret. Register this exact redirect URI:

```text
http://127.0.0.1:43187/callback
```

## Requesting the OAuth application

Register the application in the [pCloud My Apps portal](https://docs.pcloud.com/my_apps/). At the time of writing, choosing **New app** asks you to submit a request that pCloud approves manually. App approval is a prerequisite separate from the account sign-in and consent performed by this client.

Prepare the request with these values:

- **App name:** `pCloud Vault`
- **Application Type:** `Personal use`
- **Folder access:** `All folders`
- **Write access:** `Yes`
- **Website:** the actual project website or repository URL
- **Expected users:** the actual number of people who will use this personal client
- **Reason:** explain that this is a personal Windows desktop client that encrypts and decrypts files locally, uploads encrypted vault objects, and browses ordinary account files. Include the required callback URI `http://127.0.0.1:43187/callback`.

Submission does not itself create an approved OAuth app or validate live transfers. After pCloud approves the request, obtain the public client ID and ensure that the exact callback URI is registered. Do not enter a client secret in the desktop app, and never commit your client ID or account details to a repository.

Live account integration requires pCloud's approval of your application and your interactive sign-in. The prepared request, application approval, account consent, and successful upload/download verification are distinct steps; none should be reported as completed merely because the previous step succeeded.

## Connecting and validating

Configure the client ID in Settings, then choose **Connect pCloud**. The system browser opens pCloud's sign-in and consent page. The account password and two-factor authentication stay on pCloud's page. The app starts its callback listener before opening the browser, accepts one state-bound response, and closes the listener after completion or three minutes.

The application must be allowed to access your existing account files, not only its application folder. An access-denied response is shown explicitly. A successful empty listing alone does not establish that the application has full account access: verify a known pre-existing file outside the application's private folder, then verify an encrypted upload/download with the real registered application before trusting live sync.

The documented public-client flow returns a bearer token in the URL fragment. The loopback callback immediately removes that fragment from browser history and POSTs it to the local listener. A random one-time state, exact Host/Origin checks, bounded request body, CSP, and a short listener lifetime protect the callback. This is pCloud's currently documented token flow, not an authorization-code/PKCE implementation.

The callback must report one of the exact region/host pairs: US `1` / `api.pcloud.com`, or Europe `2` / `eapi.pcloud.com`. API credentials are sent in POST bodies over HTTPS. Download links are restricted to HTTPS pCloud hosts and do not receive the account token. Automatic redirects are disabled.

The account token is stored only inside a current-user Windows DPAPI record by the desktop application. Never put tokens, passwords, or account JSON into source files, shell arguments, diagnostics, or screenshots. If the token is revoked, reconnect; changing the vault password does not replace the pCloud token.

For immutable vault objects, the transport uses `nopartial=1` and `renameifexists=1`. It checks existing canonical objects and verifies a SHA-256 digest by downloading newly uploaded data before reporting them verified. File contents are already encrypted by the vault core before reaching this module. Read-back verification does not authenticate plaintext by itself; vault AEAD verification remains mandatory on reads.

The diff API provides a change cursor, and recursive listings provide full reconciliation. Advance the persisted cursor only after processing changes durably. Reconcile the full tree for first connection, invalidated cursors, and cursors older than six months. API IDs and byte sizes use 64-bit integers.

Live OAuth and account validation need a real registered client ID and interactive account approval. Unit tests cover parsing, state/replay rejection, regional routing, malicious download addresses, redacted diagnostics, and Windows DPAPI. They do not establish that a particular pCloud account or registration has the required access.

Official references: [OAuth authorization](https://docs.pcloud.com/methods/oauth_2.0/authorize.html), [global parameters](https://docs.pcloud.com/methods/intro/global_parameters.html), [file uploads](https://docs.pcloud.com/methods/file/uploadfile.html), [file links](https://docs.pcloud.com/methods/streaming/getfilelink.html), [diff](https://docs.pcloud.com/methods/general/diff.html).
