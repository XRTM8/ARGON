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

const sharedDefs = `
  <defs>
    <linearGradient id="bg" x1="128" y1="64" x2="896" y2="960" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#11050A"/>
      <stop offset="100%" stop-color="#070204"/>
    </linearGradient>
    <radialGradient id="halo" cx="50%" cy="46%" r="54%">
      <stop offset="0%" stop-color="#9E1B42" stop-opacity="0.34"/>
      <stop offset="55%" stop-color="#72142B" stop-opacity="0.15"/>
      <stop offset="100%" stop-color="#070204" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="stroke" x1="64" y1="64" x2="960" y2="960" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#E02447" stop-opacity="0.48"/>
      <stop offset="50%" stop-color="#FFFFFF" stop-opacity="0.14"/>
      <stop offset="100%" stop-color="#FFFFFF" stop-opacity="0.06"/>
    </linearGradient>
    <linearGradient id="crimson" x1="512" y1="590" x2="512" y2="816" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#E02447"/>
      <stop offset="55%" stop-color="#9E1B42"/>
      <stop offset="100%" stop-color="#72142B"/>
    </linearGradient>
  </defs>`;

const emblemPaths = `
  <circle cx="512" cy="576" r="224" fill="#FF3366" fill-opacity="0.09"/>
  <path d="M440 208H584L832 800H688L512 376L336 800H192L440 208Z" fill="#F5F6F8"/>
  <path d="M512 376L584 208L832 800H688L512 376Z" fill="#000000" fill-opacity="0.12"/>
  <path d="M512 488L560 536L512 584L464 536Z" fill="#FFFFFF"/>
  <path d="M512 509L539 536L512 563L485 536Z" fill="#FFB3C1"/>
  <path d="M478 611H546V691H605L512 816L419 691H478V611Z" fill="url(#crimson)"/>
  <path d="M512 611H546V691H605L512 816V611Z" fill="#72142B" fill-opacity="0.45"/>`;

const appIconSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  ${sharedDefs}
  <rect x="48" y="48" width="928" height="928" rx="224" fill="url(#bg)"/>
  <rect x="48" y="48" width="928" height="928" rx="224" fill="url(#halo)"/>
  <rect x="52" y="52" width="920" height="920" rx="220" stroke="url(#stroke)" stroke-width="10"/>
  ${emblemPaths}
</svg>`;

const fullCanvasIconSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" fill="none">
  ${sharedDefs}
  <rect width="1024" height="1024" fill="url(#bg)"/>
  <rect width="1024" height="1024" fill="url(#halo)"/>
  ${emblemPaths}
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
      bg_color: '#070204'
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
console.log('All Obsidian Crimson Prism ARGON icons generated and synced cleanly.');
