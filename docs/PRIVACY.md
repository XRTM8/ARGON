# ARGON Privacy Policy

**Last Updated:** November 15, 2026

ARGON is an open-source download manager designed with a strict **local-first, privacy-by-design architecture**. This Privacy Policy applies to the ARGON desktop application (Windows, Linux, macOS), the ARGON Android companion application, and the ARGON Integration browser extension (Chrome, Edge, Brave, Firefox).

---

## 1. Zero Data Collection & Zero Telemetry

ARGON does **not** collect, store, sell, or transmit any personally identifiable information (PII) or usage data to external servers or third parties.

- **No Remote Tracking:** There are no analytics SDKs (no Google Analytics, no Mixpanel, no Sentry, no telemetry tracking).
- **No User Accounts:** You do not need to register, log in, or provide an email address to use ARGON.
- **No Central Servers:** ARGON operates completely autonomously on your local hardware.

---

## 2. Browser Extension Data Handling

The **ARGON Integration** browser extension requests browser permissions solely to detect downloadable files and transfer download requests to your locally installed ARGON application:

- **Localhost Communication:** All intercepted download links, HTTP headers (such as Referer), and requested media streams are transmitted exclusively to the local loopback address (`http://127.0.0.1:41523`) or via local Native Messaging IPC (`com.otium.downloader`). No URLs or network data ever leave your computer.
- **Media Detection:** Content scripts inspect web page DOM elements (e.g. video and audio tags) strictly on the client side to surface a download action. No browsing history or page content is logged or transmitted.
- **Cookies (Optional Permission):** When enabled by the user for protected sites (such as age-restricted or authenticated video portals), session cookies are read locally and sent directly to the local ARGON desktop engine to authenticate the download stream. They are never transmitted across any external network.
- **Local Storage:** The extension uses `chrome.storage.local` exclusively to persist user preferences (such as minimum file size and site exclusion rules) and transient recovery IDs for interrupted downloads.

---

## 3. Local-Only Peer-to-Peer Transfer (Flow v2)

When using **ARGON Flow** to transfer files between devices (e.g. PC to Android):

- Transfers occur strictly over your local area network (LAN / Wi-Fi) or direct peer connection.
- All transfers are end-to-end encrypted with authenticated ephemeral keys.
- No third-party relay or cloud servers are used.

---

## 4. Updates & Security

- **Application Updates:** The desktop application periodically queries the official GitHub repository (`https://api.github.com/repos/XRTM8/ARGON/releases`) to check for newer versions. No user identifiers or telemetry data are included in these release queries.
- **Checksum Verification:** Hash verification (SHA-256) is computed 100% locally on your machine using your CPU.

---

## 5. Open Source Transparency

ARGON is open-source software under the GPL-3.0-or-later license. Its full source code, including all network communication, backend Rust crates, and extension scripts, is publicly auditable at:  
👉 **[https://github.com/XRTM8/ARGON](https://github.com/XRTM8/ARGON)**

---

## 6. Contact & Inquiries

For questions regarding this privacy policy or to report an issue, please open an issue on our GitHub repository:  
[https://github.com/XRTM8/ARGON/issues](https://github.com/XRTM8/ARGON/issues)
