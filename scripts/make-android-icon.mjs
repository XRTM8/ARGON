#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const densities = [
  ['mdpi', 1], ['hdpi', 1.5], ['xhdpi', 2], ['xxhdpi', 3], ['xxxhdpi', 4]
];

// 1. Android Adaptive Background Layer (108dp x 108dp)
// Matches the desktop PC icon obsidian gradient (#161D2A to #090C11) with subtle cobalt ambient bloom
const background = `
  <defs>
    <linearGradient id="bg" x1="14" y1="7" x2="94" y2="101" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#161D2A"/>
      <stop offset="100%" stop-color="#090C11"/>
    </linearGradient>
    <radialGradient id="highlight" cx="50%" cy="48%" r="55%">
      <stop offset="0%" stop-color="#2563EB" stop-opacity="0.14"/>
      <stop offset="100%" stop-color="#2563EB" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="108" height="108" fill="url(#bg)"/>
  <rect width="108" height="108" fill="url(#highlight)"/>`;

// 2. Android Adaptive Foreground Layer (108dp x 108dp)
// Centered at (54, 53.5) with scale(0.081) - perfectly inside the 66dp-72dp launcher safe zone
// Uses the EXACT architectural geometry, Arctic White blade, and Cobalt gradient vector from the PC icon
const foreground = `
  <defs>
    <linearGradient id="cobalt" x1="512" y1="530" x2="512" y2="796" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#60A5FA"/>
      <stop offset="100%" stop-color="#2563EB"/>
    </linearGradient>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="6" stdDeviation="8" flood-color="#000000" flood-opacity="0.45"/>
    </filter>
  </defs>
  <g transform="translate(54, 53.5) scale(0.081) translate(-512, -512)" filter="url(#shadow)">
    <path d="M464 236H560L808 788H676L512 410L348 788H216L464 236Z" fill="#F8FAFC"/>
    <path d="M478 540H546V654H598L512 788L426 654H478V540Z" fill="url(#cobalt)"/>
  </g>`;

// 3. Android 13+ Material You Monochrome Layer
const monochrome = `
  <g transform="translate(54, 53.5) scale(0.081) translate(-512, -512)">
    <path d="M464 236H560L808 788H676L512 410L348 788H216L464 236Z" fill="#FFFFFF"/>
    <path d="M478 540H546V654H598L512 788L426 654H478V540Z" fill="#FFFFFF" opacity="0.82"/>
  </g>`;

// 4. Legacy Launcher Icons (Square/Squircle & Round)
// Rendered on 18..90 viewport (72dp canvas), with the PC icon's border stroke and rounded corners
const legacySquare = `
  <defs>
    <clipPath id="squircle">
      <rect x="18" y="18" width="72" height="72" rx="17"/>
    </clipPath>
  </defs>
  <g clip-path="url(#squircle)">
    ${background}
    ${foreground}
    <rect x="18.5" y="18.5" width="71" height="71" rx="16.5" stroke="#283448" stroke-width="1" fill="none"/>
  </g>`;

const legacyRound = `
  <defs>
    <clipPath id="circle">
      <circle cx="54" cy="54" r="36"/>
    </clipPath>
  </defs>
  <g clip-path="url(#circle)">
    ${background}
    ${foreground}
    <circle cx="54" cy="54" r="35.5" stroke="#283448" stroke-width="1" fill="none"/>
  </g>`;

const adaptive = `<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
  <background android:drawable="@mipmap/ic_launcher_background"/>
  <monochrome android:drawable="@mipmap/ic_launcher_monochrome"/>
</adaptive-icon>
`;

const destinations = [
  path.join(root, 'src-tauri', 'icons', 'android'),
  path.join(root, 'src-tauri', 'gen', 'android', 'app', 'src', 'main', 'res')
];

function render(size, viewBox, contents) {
  return sharp(Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="${viewBox}">${contents}</svg>`)).png().toBuffer();
}

async function writeResource(folder, filename, bytes) {
  await Promise.all(destinations.map(async (res) => {
    const destination = path.join(res, folder);
    await mkdir(destination, { recursive: true });
    await writeFile(path.join(destination, filename), bytes);
  }));
}

export async function generateAndroidIcons() {
  for (const [density, scale] of densities) {
    const folder = `mipmap-${density}`;
    const resources = [
      ['ic_launcher_background.png', 108 * scale, '0 0 108 108', background],
      ['ic_launcher_foreground.png', 108 * scale, '0 0 108 108', foreground],
      ['ic_launcher_monochrome.png', 108 * scale, '0 0 108 108', monochrome],
      ['ic_launcher.png', 48 * scale, '18 18 72 72', legacySquare],
      ['ic_launcher_round.png', 48 * scale, '18 18 72 72', legacyRound]
    ];
    for (const [name, size, viewport, contents] of resources) {
      await writeResource(folder, name, await render(size, viewport, contents));
    }
  }
  await writeResource('mipmap-anydpi-v26', 'ic_launcher.xml', adaptive);
  await writeResource('mipmap-anydpi-v26', 'ic_launcher_round.xml', adaptive);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await generateAndroidIcons();
  console.log('Android adaptive, legacy, and monochrome icons regenerated successfully.');
}
