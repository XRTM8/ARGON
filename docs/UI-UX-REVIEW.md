# ARGON — UI/UX and browser integration review

Date: 2026-11-08. Scope: React interface, TypeScript download orchestration, Manifest V3 extension and Rust loopback bridge. Local sharing remains the existing `argon-flow/2` implementation.

## Architecture

- `src/App.tsx`: Arabic/English workspace, navigation, paste/drop intake, downloads and activity.
- `src/core/downloadManager.ts`: persistent queue, concurrency, rate limits, scheduling, native transfer lifecycle and checksum checks.
- `src-tauri/src/transfer.rs`: HTTP probe/ranges, streaming, cancellation and disk writes. `media.rs` uses yt-dlp for supported platform URLs.
- `src-tauri/src/lib.rs` and `bridge.rs`: Tauri commands and bounded loopback HTTP requests.
- `browser-extension/`: Chrome/Edge service worker, popup, media discovery and explicit page scanning.
- `flow-core/`: local encrypted sharing, pairing and clipboard delivery.

## Findings addressed

| Finding | Result |
|---|---|
| Repeated controls, glowing panels, tiny labels and crowded file cards | Muted graphite/sage interface, consistent dividers, compact file rows and progressive disclosure. No generated decorative imagery or fake transfer data. |
| A hard-coded “engine active” indicator also appeared in web preview | Preview, starting, connected and failed states now follow runtime initialization. A preview explanation appears before download intake. |
| Incomplete/queued/failed tasks were all labeled active | Separate downloading/queue counts and a neutral in-progress list. |
| No practical batch intake in the main screen | One URL per line; exact duplicates removed; invalid entries shown; maximum 100 links; signed URLs preserved. |
| Limited search and no visible multi-selection | Filename/site search, status/type filters, stable sorting and selection actions. Ctrl+F focuses search, Ctrl+N focuses intake. |
| Oversized file cards hid the relationship between files, status and speed | Desktop column alignment; mobile rows retain live speed; unknown size and finalizing states have explicit labels; progress is accessible. |
| Expired links required starting a new task | Replace an expired URL from paused/failed file details. Resume requires same origin/path plus the existing size/server-validator checks; different resources restart. Credentials and old Referer are cleared. |
| Hash calculation sounded like reference verification | SHA-256 calculation and reference matching are labeled separately. Errors are visible. |
| Disk deletion failures silently removed the task | Keep the task and report failure until deletion actually succeeds. |
| Dropped links started immediately and paste discarded additional URLs | Paste/drop populates the intake for review, including several URLs. |
| The footer implied encryption applied to all downloads | A narrow statement about local file storage replaces the blanket claim. |
| No clear extension onboarding | Dedicated setup view, folder access, browser-manager addresses and actual native listener diagnostics. |
| A TCP event emission was treated as a successful browser handoff | Native bridge waits for a saved-task acknowledgement. Storage failure/expiry rejects the handoff. Queue processing excludes pending handoffs. |
| Browser cancellation could race completion and erase history | Pause before dispatch; cancel only after confirmation; preserve browser history; resume on failure. |
| A suspended Manifest V3 worker lost its media Map | Bounded session storage, expiry and serialized per-tab updates. A recovery journal covers interrupted paused handoffs. |
| Discovery advertised unsupported HLS/DASH as downloadable | MIME/extension detection marks playlists unsupported and disables selection. |
| Capture applied everywhere and was enabled by default | Explicit opt-in, website/subdomain exclusions, source-page exclusions and a minimum-size rule. |
| Internal, incognito, flagged, extension-created or completed downloads could be captured | Eligibility checks keep them in the browser. |
| Duplicate bridge implementations had inconsistent errors | Shared request client, bounded timeouts, verified ARGON identity and explicit protocol capability. |
| Extension version text was stale and branding inconsistent | Dynamic manifest version, matching vector mark, and extension version included in the release version guard. |
| Favicon shipped a roughly 954 KB PNG | Small matching SVG favicon. |

## Comparison with IDM

IDM's official documentation describes browser integration, dynamic segmentation, resume, scheduling, queues, batch link capture and video capture. These are comparison criteria, not proof of faster transfers. Sources: [IDM features](https://www.internetdownloadmanager.com/features2.html), [IDM queues](https://idm.internetdownloadmanager.com/support/idm-scheduler/idm_queues.html), [Chrome downloads API](https://developer.chrome.com/docs/extensions/reference/api/downloads), [Chrome extension storage](https://developer.chrome.com/docs/extensions/reference/api/storage).

ARGON now offers a quieter Arabic/English interface, explicit capture controls, grouped page-link selection and a confirmed handoff path. Its existing local encrypted sharing is an additional workflow. The engine already checks byte-range behavior and remote validators, but no controlled speed comparison with IDM was run here. More connections cannot exceed the available network or server bandwidth.

## Remaining work before claiming superiority

1. **Native release gate:** compile/test the Rust/Tauri app and load the real extension in Chrome and Edge. This environment has Node but no Rust/Cargo or Visual Studio C++ toolchain. Rust changes were inspected but were not compiled or exercised against a real browser download.
2. **Authenticated and non-GET requests:** the extension forwards Referer, not browser cookies, authorization or POST bodies. Those sites need a deliberately scoped authentication design and integration tests.
3. **Streaming formats:** HLS/DASH joining, separate audio/video tracks, quality selection and muxing need a media pipeline. The current extractor chooses a combined HTTP/HTTPS format. Unsupported playlists are presented honestly.
4. **Resilience:** a saved-task response can be lost before the browser receives it. Duplicate copies remain possible during process/network failure. Exactly-once delivery would require persistent request deduplication and a status/reconciliation protocol.
5. **Download service lifecycle:** keep the native app open. Browser interception when ARGON is closed is intentionally skipped; OS service/tray background execution was not added. Android background execution remains unimplemented.
6. **Platform support:** this package targets Chrome/Edge. Firefox needs a separate manifest/background compatibility pass and installation validation. Android direct download and local-sharing behavior need device validation.
7. **Other engines:** torrent metadata exists; a P2P engine does not. FTP/MMS and recursive site capture are also absent.
8. **Scheduling:** existing time-window scheduling and one queue remain. Named queues, per-site limits, automated retry/backoff and per-download speed limits would need additional queue semantics.

No claim is made that all defects have been eliminated, that arbitrary protected videos can be downloaded, or that ARGON outperforms IDM in measured throughput.

## Validation and release procedure

- Completed here: **85 tests across 11 files passed**, TypeScript passed, Vite production build passed, JavaScript syntax checks passed, version guard passed. The built interface produced no browser warning/error logs during the final visual check.
- TypeScript project check and Vite production build.
- Vitest: download lifecycle, native-command mocks, bridge handoff failure paths and extension rules/persistence.
- Version guard includes package, Cargo, Tauri, header and extension versions.
- Manual browser review: Arabic/English, batch validation and deduplication, file details, extension setup and a 390 px phone viewport. Popup exercised using the explicitly labeled local browser-API fixture.
- Use `cargo test --manifest-path src-tauri/Cargo.toml` and build a new desktop bundle on the normal Windows development machine. An old desktop binary does not implement the new acknowledgement command/capability.
- Load the real extension and run the checklist in `browser-extension/README.md` before distribution.

For an honest speed comparison: use the same machine/network, exact same large file and server, warm/cold runs in randomized order, three repeats at 1/8/16 connections, and report median wall time, CPU/RAM, interruption recovery and SHA-256 correctness. Do not compare peak UI speed alone.
