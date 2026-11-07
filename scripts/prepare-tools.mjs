import { mkdir, readFile, writeFile, copyFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

// Windows releases ship a standalone extractor and its JavaScript runtime.
// Keep binaries out of Git; verify the extractor against upstream SHA256SUMS.
if (process.platform !== 'win32') {
  console.log('Install yt-dlp and a supported JavaScript runtime on PATH for media extraction.');
  process.exit(0);
}
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const directory = path.join(root, 'src-tauri', 'tools');
await mkdir(directory, { recursive: true });
const target = path.join(directory, 'yt-dlp.exe');
const base = 'https://github.com/yt-dlp/yt-dlp/releases/latest/download';
const get = async (url) => {
  const response = await fetch(url, { signal: AbortSignal.timeout(180000) });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response;
};
let cached;
try {
  const metadata = JSON.parse(await readFile(path.join(directory, 'extractor.json'), 'utf8'));
  const bytes = await readFile(target);
  cached = createHash('sha256').update(bytes).digest('hex') === metadata.sha256;
} catch { cached = false; }
if (!cached) {
  const checksums = await (await get(`${base}/SHA2-256SUMS`)).text();
  const hash = checksums.match(/^([a-f0-9]{64})\s+\*?yt-dlp\.exe\s*$/mi)?.[1];
  if (!hash) throw new Error('Upstream extractor checksum missing');
  const bytes = Buffer.from(await (await get(`${base}/yt-dlp.exe`)).arrayBuffer());
  if (createHash('sha256').update(bytes).digest('hex') !== hash.toLowerCase()) throw new Error('Extractor checksum mismatch');
  await writeFile(target, bytes);
  await writeFile(path.join(directory, 'extractor.json'), JSON.stringify({ sha256: hash.toLowerCase(), source: base }, null, 2));
  await writeFile(path.join(directory, 'yt-dlp-LICENSE.txt'), await (await get('https://raw.githubusercontent.com/yt-dlp/yt-dlp/master/LICENSE')).text());
}
await copyFile(process.execPath, path.join(directory, 'node.exe'));
const licenseFile = path.join(directory, 'node-LICENSE.txt');
try { await readFile(licenseFile); } catch {
  await writeFile(licenseFile, await (await get(`https://raw.githubusercontent.com/nodejs/node/${process.version}/LICENSE`)).text());
}
console.log('ARGON media tools ready (extractor checksum verified).');
