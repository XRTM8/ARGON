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
// Matches Athar/DESIGN ultra-deep wine-black base (#070204) with 3 concentric Velvet Burgundy halos
const background = `
  <rect width="108" height="108" fill="#070204"/>
  <circle cx="54" cy="54" r="46" fill="#9E1B42" fill-opacity="0.082"/>
  <circle cx="54" cy="54" r="34" fill="#72142B" fill-opacity="0.145"/>
  <circle cx="54" cy="54" r="20" fill="#9E1B42" fill-opacity="0.220"/>`;

// 2. Android Adaptive Foreground Layer (108dp x 108dp)
// Centered at (54, 54) with scale(0.081) inside the 66dp launcher safe zone
const foreground = `
  <defs>
    <linearGradient id="crimson" x1="512" y1="590" x2="512" y2="816" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#E02447"/>
      <stop offset="55%" stop-color="#9E1B42"/>
      <stop offset="100%" stop-color="#72142B"/>
    </linearGradient>
    <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
      <feDropShadow dx="0" dy="6" stdDeviation="8" flood-color="#000000" flood-opacity="0.45"/>
    </filter>
  </defs>
  <circle cx="54" cy="54" r="24" fill="#FF3366" fill-opacity="0.13"/>
  <g transform="translate(54, 54) scale(0.081) translate(-512, -512)" filter="url(#shadow)">
    <path d="M440 208H584L832 800H688L512 376L336 800H192L440 208Z" fill="#F5F6F8"/>
    <path d="M512 376L584 208L832 800H688L512 376Z" fill="#000000" fill-opacity="0.12"/>
    <path d="M512 488L560 536L512 584L464 536Z" fill="#FFFFFF"/>
    <path d="M512 509L539 536L512 563L485 536Z" fill="#FFB3C1"/>
    <path d="M478 611H546V691H605L512 816L419 691H478V611Z" fill="url(#crimson)"/>
    <path d="M512 611H546V691H605L512 816V611Z" fill="#72142B" fill-opacity="0.45"/>
  </g>`;

// 3. Android 13+ Material You Monochrome Layer
const monochrome = `
  <g transform="translate(54, 54) scale(0.081) translate(-512, -512)">
    <path d="M440 208H584L832 800H688L512 376L336 800H192L440 208Z" fill="#FFFFFF"/>
    <path d="M512 488L560 536L512 584L464 536Z" fill="#FFFFFF"/>
    <path d="M478 611H546V691H605L512 816L419 691H478V611Z" fill="#FFFFFF" opacity="0.82"/>
  </g>`;

// 4. Legacy Launcher Icons (Square/Squircle & Round)
const legacySquare = `
  <defs>
    <clipPath id="squircle">
      <rect x="18" y="18" width="72" height="72" rx="17"/>
    </clipPath>
  </defs>
  <g clip-path="url(#squircle)">
    ${background}
    ${foreground}
    <rect x="18.5" y="18.5" width="71" height="71" rx="16.5" stroke="#E02447" stroke-opacity="0.35" stroke-width="1" fill="none"/>
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
    <circle cx="54" cy="54" r="35.5" stroke="#E02447" stroke-opacity="0.35" stroke-width="1" fill="none"/>
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
