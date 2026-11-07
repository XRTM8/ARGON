# Security Policy

## Supported Versions

| Version | Supported          |
| :---    | :---:              |
| 2.0.x   | :white_check_mark: |
| < 2.0.0 | :x:                |

---

## Architectural Security Model

ARGON is designed with defensive depth to safely download files from arbitrary, untrusted internet sources:

1. **Path Traversal Mitigation (`sanitize_filename`):**
   All remote filenames derived from URLs or `Content-Disposition` headers are stripped of relative path components (`..`, `/`, `\`), null bytes (`\0`), and control characters.
2. **Windows Device Name Collision Guard:**
   Reserved MS-DOS device stems (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) are automatically prefixed (`_CON.ext`) to avoid Windows kernel locking.
3. **Command Injection Prevention:**
   System folder reveal operations use native OS APIs (via `tauri-plugin-opener`) rather than invoking shell interpreters (`cmd.exe /c start` or `sh -c`).
4. **Localhost Isolation:**
   The browser extension bridge binds exclusively to `127.0.0.1:41523` and rejects malformed payloads, never exposing remote ports or cloud sockets.
5. **Storage Quota Compaction:**
   LocalStorage task history is bounded and automatically compacted to prevent DOM storage quota exhaustion.
6. **Local Sharing:**
   Sharing starts disabled. When explicitly enabled, a separate listener accepts local IPv4 connections using Noise encryption, persistent device keys and bilateral code confirmation. Every incoming file/text still requires consent. Discovery does not grant trust. See [Flow v2](docs/FLOW-V2.md) for protocol details, bounds and tested limitations.

---

## Reporting a Vulnerability

If you discover a security vulnerability in ARGON:

1. **Do not disclose publicly** via GitHub issues, discussions, or social media.
2. Email full vulnerability reproduction steps to the project maintainer or submit a private security advisory via [GitHub Security Advisories](https://github.com/XRTM8/ARGON/security/advisories/new).
3. Include:
   - Detailed description of the potential vulnerability
   - Proof-of-concept steps or payload
   - Impact assessment
4. We acknowledge reports within 48 hours and provide patches promptly.
