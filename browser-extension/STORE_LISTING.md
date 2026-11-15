# ARGON Integration — Store Listing & Submission Guide

This document contains all official store metadata, descriptions, single-purpose justifications, and permission rationales required to publish the **ARGON Integration** extension to the **Chrome Web Store** and **Microsoft Edge Add-ons**.

---

## 1. Store Basic Metadata

- **Name:** ARGON Integration
- **Short Description (max 132 chars):**  
  *EN:* High-speed download manager integration. Intercept browser downloads and stream media directly to ARGON.  
  *AR:* ربط متكامل لمدير التنزيلات ARGON. التقاط تنزيلات المتصفح واقتناص تدفقات الفيديو والصوت بضغطة واحدة.
- **Category:** Productivity / Utilities
- **Language:** English (United States) & Arabic (العربية)
- **Privacy Policy URL:** `https://github.com/XRTM8/ARGON/blob/main/docs/PRIVACY.md`
- **Support / Website URL:** `https://github.com/XRTM8/ARGON`

---

## 2. Store Long Description

### English:
> **ARGON Integration** connects Google Chrome, Brave, and Microsoft Edge directly with your locally installed **ARGON Download Manager** for ultra-fast, multi-stream file and media downloads.
>
> **Key Capabilities:**
> - ⚡ **Automatic Download Interception:** Effortlessly hand over browser downloads to ARGON's high-speed segmented engine with zero packet loss.
> - 🎬 **Smart Video & Media Sniffer:** Automatically discovers high-definition HLS (.m3u8), DASH (.mpd), and direct video/audio streams on web pages with an instant one-click download button.
> - 🛡️ **Zero-Cloud & 100% Private:** Operates entirely locally via your machine's loopback bridge and Native Messaging Host (`127.0.0.1`). No browsing history or download links are ever transmitted to remote servers.
> - 🎛️ **Granular Per-Site Control:** Set minimum file size thresholds, exclude specific domains or subdomains, and toggle media sniffing on demand.
> - 📋 **Multi-Link Page Extractor:** Open the compact popup to scan all downloadable media and files on the active tab and send selected batches to ARGON simultaneously.
>
> *Note:* This extension requires the free, open-source ARGON desktop application installed on your computer.

### العربية:
> **إضافة آرغون (ARGON Integration)** تربط متصفحك (Chrome، Edge، Brave) مباشرة بتطبيق **ARGON لسطح المكتب** لتحقيق أقصى سرعات التنزيل المجزأ متعدد المسارات واقتناص الوسائط باحترافية.
>
> **أبرز المميزات:**
> - ⚡ **اعتراض التنزيلات التلقائي:** تسليم تنزيلات المتصفح فوراً لمحرك ARGON فائق السرعة مع الحفاظ على الكوكيز وترويسات المصادقة.
> - 🎬 **كاشف وسائط الويب الذكي:** اكتشاف مقاطع الفيديو وتدفقات HLS (.m3u8) و DASH عالية الدقة وإظهار زر تنزيل فوري بلمسة واحدة.
> - 🛡️ **خصوصية محلية 100%:** يتم نقل الروابط محلياً على جهازك فقط (`127.0.0.1`) عبر Native Messaging دون إرسال أي بايت لخوادم خارجية.
> - 🎛️ **تحكم كامل لكل موقع:** استثناء مواقع معينة، تحديد الحد الأدنى لحجم الملفات المراد التقاطها، والتحكم بالالتقاط التلقائي بسهولة.
> - 📋 **فحص روابط الصفحة بالكامل:** فتح نافذة الإضافة لفحص واستخراج كافة ملفات الوسائط في التبويب الحالي وإرسالها دفعة واحدة للتطبيق.
>
> *ملاحظة:* تتطلب الإضافة وجود تطبيق ARGON لسطح المكتب مثبتاً على جهازك.

---

## 3. Chrome Web Store Policy Justifications

### A) Single Purpose Statement:
> "The sole purpose of ARGON Integration is to detect downloadable files and media streams within the browser and seamlessly hand them over to the locally installed ARGON download manager application."

### B) Permission Justifications for Reviewers:

| Permission | Justification for Chrome Web Store Reviewers |
|---|---|
| `downloads` | Required to intercept browser downloads when automatic capture is enabled, pause the native browser download, and cancel it once ARGON successfully receives the task. |
| `nativeMessaging` | Required to communicate directly with the local ARGON desktop binary via secure standard input/output (IPC), avoiding firewall interference. |
| `webRequest` | Required to passively observe HTTP response Content-Type headers to identify direct media formats (.m3u8, .mp4, .zip, .iso) before downloading. |
| `storage` | Required to save user preferences locally (such as minimum capture size and excluded domains) and keep state recovery journals. |
| `activeTab` & `scripting` | Required to scan links on the currently active tab when the user explicitly clicks the extension icon, and to mount the video download hover button. |
| `contextMenus` | Required to add "Download with ARGON" and "Download all links with ARGON" to the browser right-click context menu. |
| `notifications` | Required to display notification banners when a handoff completes or fails. |
| `alarms` | Required to periodically retry interrupted local handoffs in Manifest V3 background service worker. |
| `cookies` *(optional)* | Only requested when the user explicitly enables "Forward session cookies for protected downloads", enabling ARGON to download authenticated media streams. |
| `<all_urls>` | Required because downloads and media files originate from arbitrary websites across the web, and the extension must detect media responses on any page the user chooses to download from. |

---

## 4. Visual Assets Checklist

- [x] **128x128 Icon:** Present at `browser-extension/icons/icon128.png`.
- [ ] **Screenshot 1 (1280x800):** Popup interface showing captured page links and connection status.
- [ ] **Screenshot 2 (1280x800):** Video download button overlay on a streaming site.
- [ ] **Promo Tile (440x280 - optional):** Clean brand graphic with "ARGON Integration".
