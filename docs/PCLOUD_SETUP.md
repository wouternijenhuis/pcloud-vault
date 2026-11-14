# Connecting pCloud

There are two ways to connect. Both keep your pCloud password on pCloud's own page.

## Sign in on the pCloud website (default)

Choose **Sign in on pCloud website**. The app opens pCloud's sign-in page (`https://my.pcloud.com`) in a separate window, where you sign in as you would in a browser, including two-step verification and social sign-in. No developer registration or approval is needed.

How it works and what it means:

- The window is **private**: it keeps no browsing profile on disk and has **no access to the app's commands**. It may only navigate to HTTPS pages.
- After you sign in, pCloud's website stores its session token in its `pcauth` cookie and your region in `locationid`. The app's Rust code reads these two cookies, clears the window's browsing data, and closes it. The token is verified with `userinfo` (trying the other region once if needed) and then stored only inside the current-user DPAPI account record.
- The token is a **full account session**, equivalent to being signed in on pCloud's website. **Forget this PC** calls pCloud's `logout` to end that session. You can also end sessions from pCloud's own account settings.
- This uses pCloud's website behavior rather than a documented developer API, so a pCloud website change can break it. If it does, use a registered OAuth app instead (below).

## Using a registered OAuth application

The desktop app can use a registered pCloud OAuth application's **public client ID** instead. It does not need or embed a client secret. Register this exact redirect URI:

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

For OAuth, expand **Use a registered pCloud app instead** in the connect dialog, enter the client ID, then choose **Continue with OAuth app**. The system browser opens pCloud's sign-in and consent page. The account password and two-factor authentication stay on pCloud's page. The app starts its callback listener before opening the browser, accepts one state-bound response, and closes the listener after completion or three minutes.

The application must be allowed to access your existing account files, not only its application folder. An access-denied response is shown explicitly. A successful empty listing alone does not establish that the application has full account access: verify a known pre-existing file outside the application's private folder, then verify an encrypted upload/download with the real registered application before trusting live sync.

The documented public-client flow returns a bearer token in the URL fragment. The loopback callback immediately removes that fragment from browser history and POSTs it to the local listener. A random one-time state, exact Host/Origin checks, bounded request body, CSP, and a short listener lifetime protect the callback. This is pCloud's currently documented token flow, not an authorization-code/PKCE implementation.

The callback must report one of the exact region/host pairs: US `1` / `api.pcloud.com`, or Europe `2` / `eapi.pcloud.com`. API credentials are sent in POST bodies over HTTPS. Download links are restricted to HTTPS pCloud hosts and do not receive the account token. Automatic redirects are disabled.

The account token is stored only inside a current-user Windows DPAPI record by the desktop application. Never put tokens, passwords, or account JSON into source files, shell arguments, diagnostics, or screenshots. If the token is revoked, reconnect; changing the vault password does not replace the pCloud token.

For immutable vault objects, the transport uses `nopartial=1` and `renameifexists=1`. It checks existing canonical objects and verifies a SHA-256 digest by downloading newly uploaded data before reporting them verified. File contents are already encrypted by the vault core before reaching this module. Read-back verification does not authenticate plaintext by itself; vault AEAD verification remains mandatory on reads.

The diff API provides a change cursor, and recursive listings provide full reconciliation. Advance the persisted cursor only after processing changes durably. Reconcile the full tree for first connection, invalidated cursors, and cursors older than six months. API IDs and byte sizes use 64-bit integers.

Live OAuth and account validation need a real registered client ID and interactive account approval. Unit tests cover parsing, state/replay rejection, regional routing, malicious download addresses, redacted diagnostics, and Windows DPAPI. They do not establish that a particular pCloud account or registration has the required access.

Official references: [OAuth authorization](https://docs.pcloud.com/methods/oauth_2.0/authorize.html), [global parameters](https://docs.pcloud.com/methods/intro/global_parameters.html), [file uploads](https://docs.pcloud.com/methods/file/uploadfile.html), [file links](https://docs.pcloud.com/methods/streaming/getfilelink.html), [diff](https://docs.pcloud.com/methods/general/diff.html).
