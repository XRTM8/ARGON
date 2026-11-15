#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const densities = [
  ['mdpi', 1], ['hdpi', 1.5], ['xhdpi', 2], ['xxhdpi', 3], ['xxxhdpi', 4]
];

const background = `
  <defs>
    <linearGradient id="base" x2="1" y2="1" gradientUnits="objectBoundingBox">
      <stop stop-color="#153971"/>
      <stop offset="1" stop-color="#0A1427"/>
    </linearGradient>
    <radialGradient id="highlight" cx="32%" cy="22%" r="64%">
      <stop stop-color="#3168BF" stop-opacity="0.48"/>
      <stop offset="1" stop-color="#3168BF" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="108" height="108" fill="url(#base)"/>
  <rect width="108" height="108" fill="url(#highlight)"/>`;

// The 108 dp layer includes room for launcher masks. Keep the mark inside the central safe zone.
const mark = (arrow) => `
  <path d="M47.8 27h12.4L79 75H66L54 41 42 75H29L47.8 27Z" fill="#F8FAFF"/>
  <path d="M49 55h10v12h7L54 81 42 67h7V55Z" fill="${arrow}"/>`;
const foreground = mark('#60CAFF');
const monochrome = mark('#F8FAFF');
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

function legacyIcon(mask) {
  return `<defs><clipPath id="launcher">${mask}</clipPath></defs><g clip-path="url(#launcher)">${background}${foreground}</g>`;
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
      ['ic_launcher.png', 48 * scale, '18 18 72 72', legacyIcon('<rect x="18" y="18" width="72" height="72" rx="16"/>')],
      ['ic_launcher_round.png', 48 * scale, '18 18 72 72', legacyIcon('<circle cx="54" cy="54" r="36"/>')]
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
}
