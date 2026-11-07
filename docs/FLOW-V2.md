# ARGON 2 — Flow integration

## Using it

1. Install ARGON 2 on both devices and connect them to the same private IPv4 network or hotspot.
2. Open Sharing and enable it on both devices. Sharing starts disabled on every launch.
3. Select a discovered device and choose Pair. If discovery is unavailable, enter the other device's displayed address and port.
4. Compare the six-digit code on both screens. Approve only if it matches, on both devices.
5. Choose files using the native picker, then send. The receiver sees each filename and size and must accept each offer.

The Clipboard page sends only the reviewed text when Send is pressed. Received text stays in memory for the current session, up to eight entries. Copying it to the OS clipboard and adding a received link to Downloads are separate user actions. Link cleaning is an optional preview and does not modify downloader URLs automatically.

On Windows, Settings lets the user choose a received-files directory. On Android, received files are inside application storage; open them from Activity to hand them to another application. Outgoing Android document selections are copied into app cache, so enough space is needed to prepare them. Old selection caches are removed at a subsequent process startup after 24 hours. Keep the Android app open while transferring.

## Implementation

`flow-core/` is an independent Rust crate, shared by Tauri desktop and Android. `src-tauri/src/flow.rs` exposes typed app commands and emits `flow-event`; `src/core/flowClient.ts` owns event subscriptions, bounded polling and UI command state. The downloader's transfer engine remains separate.

Windows uses native rfd file/folder dialogs and arboard clipboard calls. Android uses `FlowFilesPlugin.kt`, registered from `flow_mobile.rs`, for the system document picker, explicit clipboard operations and FileProvider viewing. File transfer commands take opaque selection IDs rather than arbitrary frontend paths. Selection IDs expire after 30 minutes.

The interface is implemented in React/CSS, with real native state, no fabricated devices or simulated progress. The five views share the navigation, global requests and actual activity history. The design uses black backgrounds, titanium text and dark translucent panels, with reduced-motion and reduced-transparency preferences.

## Wire protocol and trust

- Protocol identifier: `argon-flow/2`; handshake magic: `ARGONFLOW2\n`.
- TCP default: 54127, with an available-port fallback. UDP discovery: 54126. Displayed addresses include the actual listening port.
- Connections are limited to private, loopback or link-local IPv4 endpoints. Discovery advertisements are hints, not proof of identity.
- Noise `XX_25519_ChaChaPoly_BLAKE2s` authenticates possession of a persistent static private key. The peer ID is SHA-256 of its public key. A six-digit code derived from the handshake is compared by the users before both sides save trust.
- The authenticated Hello includes the listening port and whether each side remembers the peer. If either side has revoked/lost trust, both sides must approve a fresh pairing. A trust epoch prevents an in-flight approval from undoing revocation.
- Windows identities use user-scoped DPAPI. Unix/Android identities use mode-0600 files in app storage. A damaged identity produces an error instead of silently changing the paired identity.
- Control frames and binary file chunks are encrypted, including metadata, text and acknowledgements. Noise transport sequence state rejects modified/replayed ciphertext. Binary chunks are at most 48 KB.
- Pairing and incoming requests expire. Limits include eight incoming sessions, three outgoing file streams, 32 active transfers, and 20 accepted connection attempts per source IP per minute.
- UDP discovery failure leaves TCP sharing and manual address connections available. Sharing does not add firewall rules automatically.

## Files and completion

The sender hashes the complete file before offering it. Every accepted transfer records its expected size and SHA-256. Partial paths are derived from the authenticated peer, content hash and sanitized name inside `.argon-partials` under the selected directory. Partial directories/files are checked against path escape and symlink misuse.

For resume, the receiver reports its existing prefix length and checksum. The sender verifies that exact prefix; a mismatch resets the partial before any further bytes. Restarting preserves partials and device identity. Retrying requires choosing/sending the file and accepting it again; there is no unattended resume after app restart.

The receiver checks disk space, writes and syncs the file, verifies its entire length and checksum, then commits without replacing existing files. Collisions use numbered filenames. Windows uses MoveFileExW without REPLACE_EXISTING; Linux/Android uses an atomic hard link and removes the partial. Android reception therefore uses internal app storage, which supports this operation. Only then does the receiver send an encrypted completion acknowledgement; the sender requires matching size/hash before showing Complete.

The configured file limit is 50 GB, with a 16 KB UTF-8 clipboard limit. The largest automated transfer tested here is **101 MB**, not 50 GB. A disconnect after the receiver commits but before the sender receives its acknowledgement can require a retry and create a numbered duplicate; it never overwrites the saved file.

## Compatibility and validation

This intentionally replaces the old Flow wire protocol. Old Flow peers must be updated; original discovery packets and transfer requests are not silently accepted. The integration study found weaknesses in the old pairing key derivation, plaintext file transport and restart handling. Only the optional cleaner was adapted from [XRTM8/Flow](https://github.com/XRTM8/Flow), with GPL attribution in its source. The new transport uses [Snow](https://github.com/mcginty/snow). Android adapters follow the [Tauri mobile plugin APIs](https://v2.tauri.app/develop/plugins/develop-mobile/).

Automated sharing tests exercise two real loopback services and encrypted sockets. They cover bilateral pairing, rejection, one-sided revocation/re-pairing, persisted identity/trust, encrypted Unicode/empty/colliding files, a 101 MB file, cancellation, restart/resume, corrupt-prefix reset, explicit text consent, non-persistence of clipboard bodies, unsafe input rejection, listener shutdown, tamper/replay rejection and signed-link preservation. LAN broadcast across physical devices and Android OS behavior still require device testing; loopback tests cannot prove them.

Validation for this build: 68 frontend tests, 20 downloader/backend tests, and 22 sharing tests passed. The Windows release executable was opened and its Downloads, Sharing and Settings views inspected, including ARGON / BY Otium. Three real 2 MB downloads through the native browser bridge (ranges, ignored-range fallback and unknown-length streaming) all completed and matched SHA-256 `1e075c8d478ad21844e33e830a695ef03a4d2488b69ee275bd8947618bb1be1e`. The portable ZIP's executable, extension version, media tools and archive integrity were checked. The arm64 Android release build and development APK signature/alignment checks succeeded; no physical-phone test was performed.

Windows packages include the downloader's media tools and browser extension. Android supports direct downloads and the new sharing adapters; desktop yt-dlp/Node resources are excluded. There is no Android foreground service, QR pairing, cloud relay, automatic clipboard sync or legacy Flow compatibility in this version. The Android preview is development-signed; production updates need the original application's release signing key.
