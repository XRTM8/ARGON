# ARGON

## العربية

تطبيق لتنزيل الملفات ومشاركتها بين الأجهزة على الشبكة المحلية. واجهته React وTypeScript ضمن Tauri، وأجزاء التنزيل والنقل مكتوبة بـ Rust. يضم أيضًا إضافة للمتصفح ونسخة معاينة لأندرويد.

### الموجود في المستودع

- تنزيل ملفات HTTP وHTTPS مع طابور للتنزيلات، حدود للسرعة، تنزيل متوازٍ عند دعم الخادم لطلبات النطاق، واستئناف الأجزاء بعد التحقق من بيانات الملف.
- دعم روابط ذات رؤوس طلب مخصصة أو مصادقة أساسية، وفحص اختياري لبصمة SHA-256. لا تُستبدل الملفات الموجودة عند تكرار الاسم.
- استخراج روابط وسائط عبر أدوات مضمنة في نسخة Windows؛ تعتمد صيغ HLS وDASH ودمج المسارات على توفر الأدوات ومصدر الوسائط. نسخة أندرويد مخصصة للتنزيل المباشر.
- إضافة Chrome/Edge لتمرير التنزيلات واكتشاف الوسائط إلى التطبيق المحلي. تعليمات الإعداد في [browser-extension/README.md](browser-extension/README.md).
- مشاركة ملفات ونصوص بين نسختين من ARGON 2 عبر الشبكة المحلية بعد إقران الجهازين وتأكيد رمز من ستة أرقام. قبول كل ملف أو نص وارد إجراء مستقل. النقل مشفر ويتحقق من حجم الملف وبصمته قبل إتمامه.
- معاينة لتنظيف الروابط عند الطلب، وصفحات للتنزيلات والمشاركة والحافظة والنشاط والإعدادات.

### الحدود والبناء

يستخدم النقل المحلي بروتوكول `argon-flow/2`، وهو غير متوافق مع تطبيق Flow الأقدم. المشاركة تبدأ معطلة في كل تشغيل. في أندرويد يجب إبقاء التطبيق مفتوحًا أثناء النقل؛ تنفيذ النقل في الخلفية غير موجود حاليًا. معاينة Vite في المتصفح لا تنفذ تنزيلات فعلية. تفاصيل البروتوكول والاختبارات في [docs/FLOW-V2.md](docs/FLOW-V2.md).

لبناء نسخة Windows تحتاج Node.js وRust وأدوات Visual Studio C++ وWebView2:

```powershell
npm ci
npm run tauri:dev
```

للفحوص والبناء:

```powershell
npm test
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path flow-core/Cargo.toml -- --test-threads=1
npm run tauri:build -- --bundles nsis
```

`src-tauri/` يحتوي التطبيق الأصلي، و`downloader-core/` محرك التنزيل، و`flow-core/` النقل المحلي، و`browser-extension/` الإضافة. تفاصيل التحديثات في [docs/UPDATES.md](docs/UPDATES.md).

الترخيص: [GPL-3.0-or-later](LICENSE). للأدوات المضمنة تراخيصها الخاصة.

## English

ARGON is a file downloader and local device-to-device sharing application. It uses React and TypeScript in Tauri, Rust for downloads and transfers, and includes a browser extension and an Android preview.

### Repository contents

- HTTP/HTTPS downloads with a queue, speed limits, parallel ranges when supported by the server, and verified partial-file resume.
- Custom request headers, Basic authentication, optional SHA-256 verification, and collision-safe output names.
- Media URL extraction using bundled tools on Windows. HLS/DASH downloads and track merging depend on the available tools and source. Android supports direct-file downloads.
- A Chrome/Edge extension for handing downloads and detected media to the local app. See [browser-extension/README.md](browser-extension/README.md).
- Encrypted local file and text sharing between two ARGON 2 devices after pairing and comparing a six-digit code. Each incoming item requires separate acceptance. File size and SHA-256 are verified before completion.
- Optional link-cleaning previews and views for downloads, sharing, clipboard, activity, and settings.

### Limits and build

Local sharing uses `argon-flow/2` and is incompatible with the older Flow app. Sharing starts disabled on each launch. Android transfers require the app to remain open; background transfer execution is not implemented. The browser-only Vite preview does not perform real downloads. Protocol and validation details are in [docs/FLOW-V2.md](docs/FLOW-V2.md).

Windows development requires Node.js, Rust, Visual Studio C++ Build Tools, and WebView2:

```powershell
npm ci
npm run tauri:dev
```

Tests and Windows installer build:

```powershell
npm test
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path flow-core/Cargo.toml -- --test-threads=1
npm run tauri:build -- --bundles nsis
```

`src-tauri/` holds the native application, `downloader-core/` the download engine, `flow-core/` local sharing, and `browser-extension/` the extension. See [docs/UPDATES.md](docs/UPDATES.md) for update behavior.

License: [GPL-3.0-or-later](LICENSE). Bundled tools retain their own licenses.


---

للتواصل / Contact: [@MA.XRT](https://www.instagram.com/ma.xrt/)
