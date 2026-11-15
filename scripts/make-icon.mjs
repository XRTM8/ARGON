#!/usr/bin/env node
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import sharp from 'sharp';
import { generateAndroidIcons } from './make-android-icon.mjs';

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

const appIconPng = path.join(root, 'app-icon.png');
const iosMasterPng = path.join(scratchDir, 'ios-master.png');

writeFileSync(appIconPng, await sharp(Buffer.from(appIconSvg)).png().toBuffer());
writeFileSync(iosMasterPng, await sharp(Buffer.from(fullCanvasIconSvg)).png().toBuffer());

const manifestPath = path.join(scratchDir, 'icon-manifest.json');
writeFileSync(
  manifestPath,
  JSON.stringify(
    {
      default: appIconPng,
      bg_color: '#0D1117'
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

await generateAndroidIcons();

rmSync(scratchDir, { recursive: true, force: true });
console.log('All corporate ARGON icons generated and synced cleanly.');
