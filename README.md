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
- Resume when the remote size and ETag/Last-Modified still match. Otherwise restart safely from zero.
- Unknown-length streams and empty/one-byte files.
- Collision-safe file creation; existing unrelated files are not overwritten.
- Files become complete only after transfer results, disk length and an optional SHA-256 check succeed.
- A shared global speed limit and bounded concurrent download queue.
- Chrome/Edge browser extension, included in installers and portable ZIPs.
- Native social-video URL extraction through bundled yt-dlp and Node.js on Windows. The extractor selects an available combined HTTP/HTTPS format and preserves CDN request headers. Available quality depends on the source.

Browser-only Vite previews do **not** download or simulate progress. Run the native app for downloads.

Private, expired or restricted links may require authentication or a fresh direct URL. Webpages and HLS/DASH manifests are reported as unsupported rather than saved as completed media. Magnet/torrent metadata parsing is present; a P2P transfer engine is not included. Android currently supports direct files only; platform video extraction requires desktop tools.

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

Open Chrome/Edge's extension manager, enable Developer mode, choose **Load unpacked**, and select `browser-extension/`. The **Browser extension** section in ARGON includes setup instructions, folder access and bridge diagnostics. The extension offers Arabic/English controls, transient media discovery, selected page-file downloads and opt-in automatic capture with website exclusions and minimum sizes. It uses the native bridge on `127.0.0.1:41523`. Browser downloads are paused for handoff, then cancelled only after ARGON confirms saving the task; failed handoffs resume the browser download. Update the native app and extension together for this protocol. See [extension usage, permissions and validation](browser-extension/README.md).

## Android preview

With the Android SDK/NDK, Java 17 and the Rust Android target installed:

```powershell
npm run tauri:android:build -- --target aarch64 --ci
```

The build produces an unsigned release APK. Sign it with your existing release certificate for an upgrade, or a development certificate for testing. The local `arm64-preview.apk` artifact is development-signed and has not been validated on a physical phone. Android receives files into app storage, opens accepted files through FileProvider, and selects outgoing files through the system document picker. Keep the app open during transfers; background transfer execution is not implemented.

## Settings and updates

Settings display **ARGON** and **BY Otium**, and allow saving a writable download directory. Release checks use [ARGON releases](https://github.com/XRTM8/ARGON/releases). Download an available update explicitly from Settings, then open its installer from the completed downloads list. An unavailable update service is shown as an error, not as proof that the app is current.

The application identifier, storage keys and browser bridge header retain their legacy Otium values for compatibility with existing data and integrations. Historical winget manifests describe the original releases.

## License

GPL-3.0-or-later. Bundled media components retain their upstream licenses, included in `tools/`.
