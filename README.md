# ARGON

**BY Otium** · Downloads, encrypted local sharing and clipboard delivery in one Arabic/English workspace.

## ARGON 2

The Arabic/English interface uses graphite surfaces, a restrained sage accent and aligned file rows. Downloads, Sharing, Clipboard, Activity, Browser Extension and Settings share one navigation panel, with a bottom navigation bar on phones. Downloads support reviewed batch intake, search/status/type filters, sorting, multi-selection and replacing expired links. See the [UI/UX review and comparison criteria](docs/UI-UX-REVIEW.md).

- Native file selection and encrypted device-to-device transfers, with a six-digit pairing code confirmed on both screens.
- Explicit acceptance for every received file or text; no unattended clipboard synchronization.
- SHA-256 verification for every file, verified partial-file resume after restarting, collision-safe saves and receiver acknowledgements.
- A completed download can be selected for local sharing. Received links can be added to the downloader explicitly.
- Link cleaning is previewed on request. Signed/authenticated URLs are preserved.

Both devices need **ARGON 2**. The new `argon-flow/2` protocol does not communicate with the older Flow application. See [protocol, usage and validation details](docs/FLOW-V2.md) and the original [integration study](research/ARGON-FLOW-STUDY.md).

## Downloads

- HTTP/HTTPS files, redirects, custom request headers and URLs with Basic authentication.
- Parallel ranges verified using a real GET probe. Servers that ignore ranges fall back to a single stream after all other writers have stopped.
- Resume verified partial files when the effective URL, length and strong ETag match. Older unverified partial files restart safely. A single stream is used for servers without a strong validator.
- Unknown-length streams and empty/one-byte files.
- Collision-safe file creation; existing unrelated files are not overwritten.
- Files become complete only after transfer results, disk length and an optional SHA-256 check succeed.
- A shared global speed limit and bounded concurrent download queue.
- Chrome/Edge browser extension, included in installers and portable ZIPs.
- Native social-video URL extraction through bundled yt-dlp and Node.js on Windows. With FFmpeg available, the desktop app can download unprotected HLS/DASH streams and merge separate audio and video tracks. Available quality depends on the source.

Browser-only Vite previews do **not** download or simulate progress. Run the native app for downloads.

Private, expired or restricted links may require authentication or a fresh direct URL. Magnet/torrent metadata parsing is present. Android supports direct files and uses the desktop app for downloading HLS/DASH streams.

## Build and run on Windows

Prerequisites: Node.js, a current stable Rust toolchain, Visual Studio C++ Build Tools and WebView2.

```powershell
npm ci
npm run tauri:dev
```

`tauri:dev` and `tauri:build` prepare the Windows media resources automatically. The first preparation downloads yt-dlp from its official release and verifies SHA-256 before use. The build's Node.js runtime and third-party licenses travel with the app. Subsequent builds reuse the verified extractor. To refresh it, remove `src-tauri/tools/extractor.json` and run `npm run prepare:tools`.

```powershell
npm test
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path flow-core/Cargo.toml -- --test-threads=1
npm run version:check
npm run tauri:build -- --bundles nsis
```

An optional live HTTPS smoke test is available:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml public_https_download -- --ignored
```

The installer is generated under `src-tauri/target/release/bundle/nsis/`. A portable distribution must include `ARGON.exe`, `tools/` and `browser-extension/` together. Do not distribute only the executable if video extraction and extension installation are needed. Linux/macOS development requires yt-dlp and a supported JavaScript runtime on PATH.

## Browser extension

Open Chrome/Edge's extension manager, enable Developer mode, choose **Load unpacked**, and select `browser-extension/`. The **Browser extension** section in ARGON includes setup instructions, connection diagnostics and a test file. The extension offers Arabic/English controls, transient media discovery, selected page-file downloads and opt-in automatic capture with website exclusions and minimum sizes. It uses the native bridge on `127.0.0.1:41523`. Automatic capture records a saved task before transferring responsibility to ARGON. Update the native app and extension together to use staged recovery. See [extension usage, permissions and validation](browser-extension/README.md).

## Android preview

With the Android SDK/NDK, Java 17 and the Rust Android target installed:

```powershell
npm run tauri:android:build -- --target aarch64 --ci
```

The build produces an unsigned release APK. Sign it with your existing release certificate for an upgrade, or a development certificate for testing. The local `arm64-preview.apk` artifact is development-signed and has not been validated on a physical phone. Android receives files into app storage, opens accepted files through FileProvider, and selects outgoing files through the system document picker. Keep the app open during transfers; background transfer execution is not implemented.

## Settings and updates

Settings offer optional automatic update checking and downloading, **off by default**. For the Windows x64 installer, checks run at startup and every six hours when enabled. Packages must pass the embedded public-key signature check and carry the same signed version announced by the release feed. Installation requires an explicit click after downloads and sharing finish; it upgrades the existing per-user installation and restarts ARGON. A single-instance guard prevents a second running app. Failed checks remain errors, rather than claiming the app is current.

Portable copies never invoke the NSIS updater: Settings offers the matching ZIP, verified by its published SHA-256. Close the old copy and extract the new ZIP into a separate folder. Install **2.2.0 once manually** if upgrading from 2.1.0, then enable automatic updates in Settings if desired. See [update behavior and release validation](docs/UPDATES.md).

The application identifier, storage keys and browser bridge header retain their legacy Otium values for compatibility with existing data and integrations. Historical winget manifests describe the original releases.

## License

GPL-3.0-or-later. Bundled media components retain their upstream licenses, included in `tools/`.
