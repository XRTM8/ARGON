// Local integration fixture for exercising the packaged native app.
import http from 'node:http';
import { createHash } from 'node:crypto';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { setTimeout as delay } from 'node:timers/promises';
const data = Buffer.alloc(2 * 1024 * 1024);
for (let i = 0; i < data.length; i++) data[i] = i % 251;
const MiB = 1024 * 1024;
const benchmark = process.argv.includes('--benchmark');
const cases = new Map([
  ['/small.bin', { size: 256 * 1024 }],
  ['/large.bin', { size: 1024 * MiB }],
  ['/slow.bin', { size: 100 * MiB, latency: 150 }],
  ['/limited.bin', { size: 50 * MiB, limit: 2 }],
  ['/interrupted.bin', { size: 50 * MiB, interrupt: 2 * MiB }]
]);
const active = new Map();
const interrupted = new Set();

function* bytes(start, end) {
  for (let offset = start; offset <= end;) {
    const from = offset % data.length;
    const length = Math.min(64 * 1024, end - offset + 1, data.length - from);
    yield data.subarray(from, from + length);
    offset += length;
  }
}

async function serveBenchmark(request, response) {
  const url = new URL(request.url, 'http://127.0.0.1');
  const scenario = cases.get(url.pathname);
  if (!scenario || !['GET', 'HEAD'].includes(request.method)) {
    response.writeHead(404).end();
    return;
  }
  const inFlight = active.get(url.pathname) || 0;
  if (scenario.limit && inFlight >= scenario.limit) {
    response.writeHead(503, { 'Retry-After': '1' }).end();
    return;
  }
  const tag = `"fixture-${scenario.size}"`;
  const range = !request.headers['if-range'] || request.headers['if-range'] === tag
    ? /^bytes=(\d+)-(\d*)$/.exec(request.headers.range || '') : null;
  const from = range ? Number(range[1]) : 0;
  const end = range?.[2] ? Math.min(Number(range[2]), scenario.size - 1) : scenario.size - 1;
  if (from > end) {
    response.writeHead(416, { 'Content-Range': `bytes */${scenario.size}` }).end();
    return;
  }
  active.set(url.pathname, inFlight + 1);
  try {
    if (scenario.latency) await delay(scenario.latency);
    const run = url.searchParams.get('run') || 'default';
    const drop = request.method === 'GET' && scenario.interrupt && !interrupted.has(run)
      && end - from + 1 > scenario.interrupt;
    response.writeHead(range ? 206 : 200, {
      'Content-Type': 'application/octet-stream', 'Content-Length': end - from + 1,
      'Accept-Ranges': 'bytes', ETag: tag,
      ...(drop ? { Connection: 'close' } : {}),
      ...(range ? { 'Content-Range': `bytes ${from}-${end}/${scenario.size}` } : {})
    });
    if (request.method === 'HEAD') {
      response.end();
      return;
    }
    if (drop) {
      interrupted.add(run);
      // Keep the declared length so a client detects the missing tail.
      for (const chunk of bytes(from, from + scenario.interrupt - 1)) response.write(chunk);
      response.end();
    } else {
      await pipeline(Readable.from(bytes(from, end)), response);
    }
  } finally {
    active.set(url.pathname, active.get(url.pathname) - 1);
  }
}

const server = http.createServer((request, response) => {
  if (benchmark) {
    void serveBenchmark(request, response).catch(() => response.destroy());
    return;
  }
  if (request.url === '/denied') { response.writeHead(403); response.end(); return; }
  const match = request.headers.range?.match(/^bytes=(\d+)-(\d+)$/);
  const range = request.url === '/range.bin' || (request.url === '/fallback.bin' && request.headers.range === 'bytes=0-0');
  if (range && match) {
    const start = Number(match[1]), end = Number(match[2]);
    if (start > end || end >= data.length) { response.writeHead(416, { 'Content-Range': `bytes */${data.length}` }); response.end(); return; }
    response.writeHead(206, { 'Content-Type': 'application/octet-stream', 'Content-Length': end-start+1, 'Content-Range': `bytes ${start}-${end}/${data.length}`, ETag: '"fixture-v1"' });
    response.end(data.subarray(start, end+1)); return;
  }
  response.writeHead(200, { 'Content-Type': 'application/octet-stream', ...(request.url === '/unknown.bin' ? {} : { 'Content-Length': data.length }) });
  response.write(data.subarray(0, data.length / 2));
  response.end(data.subarray(data.length / 2));
});
server.listen(Number(process.env.ARGON_SMOKE_PORT) || 0, '127.0.0.1', () => {
  const port = server.address().port;
  if (!benchmark) {
    console.log(JSON.stringify({ port, bytes: data.length, sha256: createHash('sha256').update(data).digest('hex') }));
    return;
  }
  const scenarios = Object.fromEntries([...cases].map(([path, { size }]) => {
    const hash = createHash('sha256');
    for (const chunk of bytes(0, size - 1)) hash.update(chunk);
    return [path, { bytes: size, sha256: hash.digest('hex') }];
  }));
  console.log(JSON.stringify({ port, scenarios }));
});
