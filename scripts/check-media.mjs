// Optional live metadata smoke test, without downloading the media body.
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const extractor = path.join(root, 'src-tauri/tools/yt-dlp.exe');
const runtime = path.join(root, 'src-tauri/tools/node.exe');
const child = spawn(extractor, ['--ignore-config', '--no-playlist', '--no-warnings', '--skip-download', '--dump-single-json', '--socket-timeout', '15', '--retries', '1', '--js-runtimes', `node:${runtime}`, '--format', 'best[protocol=https]/best[protocol=http]', '--', process.argv[2] || 'https://www.youtube.com/watch?v=aqz-KE-bpKQ'], { windowsHide: true });
let stdout = '', stderr = '';
const timer = setTimeout(() => child.kill(), 90000);
child.stdout.on('data', (data) => { stdout += data; });
child.stderr.on('data', (data) => { stderr += data; });
child.on('error', (error) => { clearTimeout(timer); console.error(error.message); process.exitCode = 1; });
child.on('close', (code) => {
  clearTimeout(timer);
  if (code !== 0) { console.error(stderr.slice(0, 1500)); process.exitCode = 1; return; }
  const result = JSON.parse(stdout);
  console.log(JSON.stringify({ title: result.title, extension: result.ext, protocol: result.protocol, streamHost: new URL(result.url).hostname }));
});
