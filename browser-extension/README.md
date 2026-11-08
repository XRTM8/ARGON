# ARGON browser extension

Manifest V3 integration for Chrome and Edge, versioned with the desktop app. Load `browser-extension/` with **Load unpacked** in `chrome://extensions` or `edge://extensions`. ARGON also includes this folder in desktop bundles. Open **Browser extension** inside ARGON for setup steps and native bridge diagnostics.

## Features

- Arabic/English popup with an explicit connection check and contextual error messages.
- Send a single URL, several URLs (one per line), the current page URL, or a link/media item from its context menu.
- Detect direct audio/video from response MIME types as well as file extensions. Media survives Manifest V3 worker suspension in `storage.session`, with a 40-item tab limit and a 30-minute expiry.
- Scan the active page on request, review up to 100 file links, and choose which ones to send.
- Automatic capture is **off on fresh installs**. Existing saved preferences are preserved. Exclude a website (including its subdomains) or set a minimum file size. Source-page exclusions also apply to CDN downloads.
- Incognito, flagged, already paused, completed, extension-created, non-HTTP and unsupported playlist downloads remain in the browser.

## Confirmed handoff

The extension connects only to `http://127.0.0.1:41523`. `/ping` must identify ARGON and advertise `persisted-handoff`; older app builds must be updated together with the extension.

For automatic capture, the extension checks the latest browser download state, records a recovery entry, pauses the browser download, and posts to `/add`. The native bridge emits a uniquely identified request and waits up to four seconds. The frontend saves the task to local storage before acknowledging it through Tauri. Only the acknowledged response allows the extension to cancel the paused browser download. Failed handoffs resume it. Browser download history is retained. A recovery journal handles interrupted worker handoffs on its next startup; it contains browser download IDs and timestamps, not file URLs or credentials.

This does not guarantee exactly-once delivery across process or network failures: a response can be lost after ARGON has saved a task. Prefer a duplicate over losing the only copy. No native transfer is started before frontend acknowledgement completes.

## Permissions and storage

`downloads` controls browser handoffs; `contextMenus` adds explicit send actions; `storage` stores preferences, transient media and recovery records; `notifications` reports handoff results; `webRequest` observes media response headers; `activeTab` and `scripting` scan the selected page only after a click. HTTP/HTTPS host access is needed to observe media, plus loopback access to the bridge. No content script is injected automatically, no analytics service is used, and browser cookies are not collected.

`<all_urls>` is currently declared for media discovery. Sites requiring cookies, POST bodies, one-time tokens or private login state may not work with a replayed URL. Referer is forwarded when available; browser cookies and authorization are not copied. HLS/DASH playlists, blob URLs, DRM streams, magnet/torrent transfers and Firefox installation are not supported by this package.

## Validation

Run `npm test`. The tests cover capture rules, MIME detection, exact signed-URL preservation, worker storage persistence, saved-task acknowledgement, failed handoff recovery and older-protocol rejection. Frontend lifecycle tests verify persistence failure and acknowledgement expiry.

For visual review, run Vite and open `/scripts/extension-preview.html`. This loads the actual popup with clearly marked simulated browser APIs; it does not test a real installed extension or native bridge.

Before shipping, load the real extension in Chrome and Edge and test against a newly built ARGON app: bridge offline, write/storage failure, normal file, large resumable file, redirect, excluded CDN source, worker restart, private/incognito transfer, unsupported playlist and interrupted handoff. Unit tests and the visual fixture do not replace those integration checks.
