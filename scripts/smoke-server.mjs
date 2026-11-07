// Local integration fixture for exercising the packaged native app.
import http from 'node:http';
import { createHash } from 'node:crypto';
const data = Buffer.alloc(2 * 1024 * 1024);
for (let i = 0; i < data.length; i++) data[i] = i % 251;
const server = http.createServer((request, response) => {
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
server.listen(Number(process.env.ARGON_SMOKE_PORT) || 0, '127.0.0.1', () => console.log(JSON.stringify({ port: server.address().port, bytes: data.length, sha256: createHash('sha256').update(data).digest('hex') })));
