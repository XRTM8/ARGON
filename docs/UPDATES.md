# Application updates

ARGON 2.2.0 adds signed updates for the Windows x64 per-user NSIS installation. Automatic checks and downloads are optional and default to off. The app checks at startup and every six hours only after opting in. Installation always requires a click, waits for downloads and sharing to finish, saves the download queue and restarts the app through the official Tauri updater.

## Package compatibility

- The registered install location and running executable directory must match, with the NSIS uninstaller present. Other Windows copies are treated as Portable.
- The application identifier and current-user installer scope remain unchanged. Downgrades are disabled. Only stable versions newer than the current app are accepted.
- Only `windows-x86_64-nsis` packages with the exact official versioned ARGON installer URL are accepted. Portable copies use a matching ZIP and never launch this installer.
- The updater serializes checks, downloads and installs. A cached package must match its version, signature and SHA-256, and pass the official signature verification before it becomes ready or affects active services. The signature's trusted comment must bind it to the announced version.
- A single-instance guard focuses the existing window. The download bridge rejects handoffs during installation, and failed installation releases the queue lock.

Portable updates are explicit downloads; close the previous app and extract into a separate folder. The ZIP's published SHA-256 detects download corruption; it is not a substitute for the installed updater's public-key authentication. Android has no automatic in-app installation in this release.

## Release verification

CI tests the frontend and native download/sharing engines before building. The Windows build uses the existing `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repository secrets. Publishing fails if the generated installer signature does not verify with ARGON's embedded public key or does not contain the built version. Installer, signature, complete Portable ZIP, extension ZIP, feed and checksums are uploaded to a draft and checked before publication.

Updater artifact signing is separate from Windows Authenticode code signing. The first upgrade from 2.1.0 to 2.2.0 is manual because the previous version lacks this updater implementation.
