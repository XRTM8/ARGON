# ARGON browser extension

Manifest V3 integration for Chrome, Brave and Edge, versioned with the desktop app. Extract the extension ZIP into a permanent folder, then use **Load unpacked** in `chrome://extensions`, `brave://extensions` or `edge://extensions`. ARGON also includes this folder in desktop bundles. After replacing an existing extension's files, click its reload button in the extension manager and reload open media pages. Run ARGON to receive downloads.

## Features

- Compact 320px Arabic/English popup with connection status and per-file download buttons. Advanced controls stay collapsed.
- Automatically scan the current page when opening the popup. Paste a URL or send a link/media item from its context menu.
- Detect direct audio/video from response MIME types as well as file extensions. Media survives Manifest V3 worker suspension in `storage.session`, with a 40-item tab limit and a 30-minute expiry.
- Observe media while a page loads and plays. A small download button appears over supported video on hover or playback; disable it in Options or exclude the website. Supported social-video pages are sent to the desktop extractor instead of downloading isolated audio/video fragments.
- Scan up to 100 page links and show up to 40 combined supported results; refresh the list after starting playback if necessary.
- Automatic capture is **off on fresh installs**. Existing saved preferences are preserved. Exclude a website (including its subdomains) or set a minimum file size. Source-page exclusions also apply to CDN downloads.
- Incognito, flagged, already paused, completed, extension-created, non-HTTP and unsupported playlist downloads remain in the browser.

## Confirmed handoff

The extension connects to `http://127.0.0.1:41523`. `/ping` identifies ARGON and advertises the supported handoff and media capabilities. Run the test download on the **Browser extension** page after enabling capture.

With `staged-handoff`, automatic capture records a request ID before pausing the browser download. ARGON saves a prepared task under that ID, then starts it after the extension confirms the handoff. The browser cancels its own download after confirmation. A rejected preparation restores the browser download. If confirmation is uncertain, the extension keeps the browser paused and checks ARGON again when the worker restarts or its recovery alarm fires. The recovery journal stores identifiers and state. Browser history is retained.

Older ARGON versions that advertise `persisted-handoff` use a single acknowledged request. If the response is lost in that mode, a second copy can result from browser recovery.

## Permissions and storage

`downloads` controls browser handoffs; `contextMenus` adds explicit send actions; `storage` stores preferences, transient media and recovery records; `alarms` retries interrupted handoffs; `notifications` reports handoff results; `webRequest` observes media response headers; `activeTab` and `scripting` scan the selected page when opening the popup. The optional `cookies` permission forwards cookies for the download URL when the user enables it. The extension sends data directly to the local bridge.

`<all_urls>` is declared for media discovery. The extension forwards Referer when available. Downloads that depend on original POST bodies or one-time tokens can require a fresh direct link. Unprotected HLS/DASH streams are available when the bridge advertises FFmpeg support. Firefox has a separate manifest for manual testing.

## Validation

Run `npm test`. The tests cover capture rules, MIME detection, exact signed-URL preservation, worker storage persistence, saved-task acknowledgement, failed handoff recovery and older-protocol rejection. Frontend lifecycle tests verify persistence failure and acknowledgement expiry.

For visual review, run Vite and open `/scripts/extension-preview.html`. This loads the actual popup with clearly marked simulated browser APIs; it does not test a real installed extension or native bridge.

Before shipping, load the real extension in Chrome and Edge and test against a newly built ARGON app: bridge offline, write/storage failure, normal file, large resumable file, redirect, excluded CDN source, worker restart, private/incognito transfer, unsupported playlist and interrupted handoff. Unit tests and the visual fixture do not replace those integration checks.
