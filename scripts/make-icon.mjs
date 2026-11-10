#!/usr/bin/env node
import { copyFileSync, cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import sharp from 'sharp';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const scratchDir = path.join(root, '.icon-build-tmp');
mkdirSync(scratchDir, { recursive: true });

const appIconSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  <defs>
    <linearGradient id="bg" x1="128" y1="64" x2="896" y2="960" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#161D2A"/>
      <stop offset="100%" stop-color="#090C11"/>
    </linearGradient>
    <linearGradient id="cobalt" x1="512" y1="530" x2="512" y2="796" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#60A5FA"/>
      <stop offset="100%" stop-color="#2563EB"/>
    </linearGradient>
  </defs>
  <rect x="48" y="48" width="928" height="928" rx="224" fill="url(#bg)"/>
  <rect x="52" y="52" width="920" height="920" rx="220" stroke="#283448" stroke-width="8"/>
  <path d="M464 236H560L808 788H676L512 410L348 788H216L464 236Z" fill="#F8FAFC"/>
  <path d="M478 540H546V654H598L512 788L426 654H478V540Z" fill="url(#cobalt)"/>
</svg>`;

const fullCanvasIconSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  <defs>
    <linearGradient id="bg" x1="128" y1="64" x2="896" y2="960" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#161D2A"/>
      <stop offset="100%" stop-color="#090C11"/>
    </linearGradient>
    <linearGradient id="cobalt" x1="512" y1="530" x2="512" y2="796" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#60A5FA"/>
      <stop offset="100%" stop-color="#2563EB"/>
    </linearGradient>
  </defs>
  <rect width="1024" height="1024" fill="url(#bg)"/>
  <path d="M464 236H560L808 788H676L512 410L348 788H216L464 236Z" fill="#F8FAFC"/>
  <path d="M478 540H546V654H598L512 788L426 654H478V540Z" fill="url(#cobalt)"/>
</svg>`;

const androidBgSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  <defs>
    <linearGradient id="bg" x1="128" y1="64" x2="896" y2="960" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#161D2A"/>
      <stop offset="100%" stop-color="#090C11"/>
    </linearGradient>
  </defs>
  <rect width="1024" height="1024" fill="url(#bg)"/>
</svg>`;

const androidFgSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  <defs>
    <linearGradient id="cobalt" x1="512" y1="530" x2="512" y2="796" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#60A5FA"/>
      <stop offset="100%" stop-color="#2563EB"/>
    </linearGradient>
  </defs>
  <path d="M464 236H560L808 788H676L512 410L348 788H216L464 236Z" fill="#F8FAFC"/>
  <path d="M478 540H546V654H598L512 788L426 654H478V540Z" fill="url(#cobalt)"/>
</svg>`;

const androidMonoSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  <path d="M464 236H560L808 788H676L512 410L348 788H216L464 236Z" fill="#FFFFFF"/>
  <path d="M478 540H546V654H598L512 788L426 654H478V540Z" fill="#FFFFFF"/>
</svg>`;

const appIconPng = path.join(root, 'app-icon.png');
const iosMasterPng = path.join(scratchDir, 'ios-master.png');
const bgPng = path.join(scratchDir, 'android-bg.png');
const fgPng = path.join(scratchDir, 'android-fg.png');
const monoPng = path.join(scratchDir, 'android-mono.png');

writeFileSync(appIconPng, await sharp(Buffer.from(appIconSvg)).png().toBuffer());
writeFileSync(iosMasterPng, await sharp(Buffer.from(fullCanvasIconSvg)).png().toBuffer());
writeFileSync(bgPng, await sharp(Buffer.from(androidBgSvg)).png().toBuffer());
writeFileSync(fgPng, await sharp(Buffer.from(androidFgSvg)).png().toBuffer());
writeFileSync(monoPng, await sharp(Buffer.from(androidMonoSvg)).png().toBuffer());

const manifestPath = path.join(scratchDir, 'icon-manifest.json');
writeFileSync(
  manifestPath,
  JSON.stringify(
    {
      default: appIconPng,
      bg_color: '#0D1117',
      android_bg: bgPng,
      android_fg: fgPng,
      android_fg_scale: 74,
      android_monochrome: monoPng
    },
    null,
    2
  )
);

console.log('Generating Tauri platform icons via @tauri-apps/cli...');
const tauriCli = path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
execFileSync(process.execPath, [tauriCli, 'icon', manifestPath], { stdio: 'inherit', cwd: root });

// Generate flat iOS icons cleanly without double-squircle clipping
const iosDir = path.join(root, 'src-tauri', 'icons', 'ios');
if (existsSync(iosDir)) {
  const iosSizes = [
    ['AppIcon-20x20@1x.png', 20],
    ['AppIcon-20x20@2x-1.png', 40],
    ['AppIcon-20x20@2x.png', 40],
    ['AppIcon-20x20@3x.png', 60],
    ['AppIcon-29x29@1x.png', 29],
    ['AppIcon-29x29@2x-1.png', 58],
    ['AppIcon-29x29@2x.png', 58],
    ['AppIcon-29x29@3x.png', 87],
    ['AppIcon-40x40@1x.png', 40],
    ['AppIcon-40x40@2x-1.png', 80],
    ['AppIcon-40x40@2x.png', 80],
    ['AppIcon-40x40@3x.png', 120],
    ['AppIcon-60x60@2x.png', 120],
    ['AppIcon-60x60@3x.png', 180],
    ['AppIcon-76x76@1x.png', 76],
    ['AppIcon-76x76@2x.png', 152],
    ['AppIcon-83.5x83.5@2x.png', 167],
    ['AppIcon-512@2x.png', 1024]
  ];
  for (const [name, size] of iosSizes) {
    const buf = await sharp(iosMasterPng).resize(size, size).png().toBuffer();
    writeFileSync(path.join(iosDir, name), buf);
  }
}

// Generate Browser Extension icons (16, 48, 128)
const extIconDir = path.join(root, 'browser-extension', 'icons');
mkdirSync(extIconDir, { recursive: true });
for (const size of [16, 48, 128]) {
  const buf = await sharp(appIconPng).resize(size, size, { kernel: sharp.kernel.lanczos3 }).png().toBuffer();
  writeFileSync(path.join(extIconDir, `icon${size}.png`), buf);
}

// Tauri CLI writes Android icons directly into src-tauri/gen/android/app/src/main/res when gen/android exists.
// Clean stale Android Studio defaults, generate crisp legacy/round mipmaps too, and mirror into src-tauri/icons/android.
const genAndroidRes = path.join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'res');
const tauriAndroidIcons = path.join(root, 'src-tauri', 'icons', 'android');

const mipmapSizes = [
  ['mipmap-mdpi', 48],
  ['mipmap-hdpi', 72],
  ['mipmap-xhdpi', 96],
  ['mipmap-xxhdpi', 144],
  ['mipmap-xxxhdpi', 192]
];

if (existsSync(genAndroidRes)) {
  for (const stale of [
    path.join(genAndroidRes, 'drawable-v24', 'ic_launcher_foreground.xml'),
    path.join(genAndroidRes, 'drawable', 'ic_launcher_background.xml')
  ]) {
    if (existsSync(stale)) rmSync(stale);
  }

  for (const [folder, size] of mipmapSizes) {
    const dir = path.join(genAndroidRes, folder);
    mkdirSync(dir, { recursive: true });
    // Square launcher icon for legacy Android (< API 26)
    const squareBuf = await sharp(appIconPng).resize(size, size, { kernel: sharp.kernel.lanczos3 }).png().toBuffer();
    writeFileSync(path.join(dir, 'ic_launcher.png'), squareBuf);
    // Circular masked icon for Android 7.1 round icon launchers
    const circleMask = Buffer.from(
      `<svg width="${size}" height="${size}" viewBox="0 0 ${size} ${size}" xmlns="http://www.w3.org/2000/svg"><circle cx="${size / 2}" cy="${size / 2}" r="${size / 2}" fill="#fff"/></svg>`
    );
    const roundBuf = await sharp(iosMasterPng)
      .resize(size, size, { kernel: sharp.kernel.lanczos3 })
      .composite([{ input: circleMask, blend: 'dest-in' }])
      .png()
      .toBuffer();
    writeFileSync(path.join(dir, 'ic_launcher_round.png'), roundBuf);
  }

  const androidBgXml = `<?xml version="1.0" encoding="utf-8"?>\n<resources>\n  <color name="ic_launcher_background">#0D1117</color>\n</resources>\n`;
  writeFileSync(path.join(genAndroidRes, 'values', 'ic_launcher_background.xml'), androidBgXml);

  const adaptiveXml = `<?xml version="1.0" encoding="utf-8"?>\n<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>\n  <background android:drawable="@mipmap/ic_launcher_background"/>\n  <monochrome android:drawable="@mipmap/ic_launcher_monochrome"/>\n</adaptive-icon>\n`;
  mkdirSync(path.join(genAndroidRes, 'mipmap-anydpi-v26'), { recursive: true });
  writeFileSync(path.join(genAndroidRes, 'mipmap-anydpi-v26', 'ic_launcher.xml'), adaptiveXml);
  writeFileSync(path.join(genAndroidRes, 'mipmap-anydpi-v26', 'ic_launcher_round.xml'), adaptiveXml);

  // Mirror cleanly into src-tauri/icons/android
  rmSync(tauriAndroidIcons, { recursive: true, force: true });
  mkdirSync(tauriAndroidIcons, { recursive: true });
  for (const [folder] of mipmapSizes) {
    cpSync(path.join(genAndroidRes, folder), path.join(tauriAndroidIcons, folder), { recursive: true });
  }
  cpSync(path.join(genAndroidRes, 'mipmap-anydpi-v26'), path.join(tauriAndroidIcons, 'mipmap-anydpi-v26'), { recursive: true });
  mkdirSync(path.join(tauriAndroidIcons, 'values'), { recursive: true });
  copyFileSync(path.join(genAndroidRes, 'values', 'ic_launcher_background.xml'), path.join(tauriAndroidIcons, 'values', 'ic_launcher_background.xml'));
}

rmSync(scratchDir, { recursive: true, force: true });
console.log('All corporate ARGON icons generated and synced cleanly.');
