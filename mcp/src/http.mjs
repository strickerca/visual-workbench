import http from 'node:http';
import { timingSafeEqual } from 'node:crypto';
import { createMcpHandler } from '@modelcontextprotocol/server';
import { LIMITS, Slots, encoded, json, requireThat } from './bounds.mjs';

export function authorize(request, port, token) {
  requireThat(request.socket.remoteAddress === '127.0.0.1', 'forbidden');
  requireThat(request.url === '/mcp' && request.headers.host === `127.0.0.1:${port}`, 'forbidden');
  // A missing Origin is required for native clients. A present Origin must be
  // exactly this loopback authority: null, localhost aliases and subdomains fail.
  const origin = request.headers.origin;
  requireThat(origin === undefined || origin === `http://127.0.0.1:${port}`, 'forbidden');
  const supplied = Buffer.from(request.headers.authorization ?? '');
  const expected = Buffer.from(`Bearer ${token}`);
  requireThat(supplied.length === expected.length && timingSafeEqual(supplied, expected), 'forbidden');
  requireThat(!request.headers['content-encoding'] && !request.headers.upgrade, 'invalid_request');
  const names = request.rawHeaders.filter((_, i) => i % 2 === 0).map(x => x.toLowerCase());
  for (const name of ['authorization', 'host', 'origin', 'content-length', 'mcp-protocol-version'])
    requireThat(names.filter(x => x === name).length <= 1, 'invalid_request');
}
async function body(request) {
  const chunks = []; let size = 0;
  for await (const chunk of request) {
    size += chunk.length; requireThat(size <= LIMITS.request, 'request_too_large'); chunks.push(chunk);
  }
  const bytes = Buffer.concat(chunks, size); return { bytes, parsed: json(bytes) };
}
export async function listenHttp(make, token, port) {
  requireThat(/^[a-f0-9]{64}$/.test(token) && Number.isInteger(port) && port >= 0 && port <= 65535);
  const handler = createMcpHandler(make, { legacy: 'stateless', responseMode: 'json', maxRequestBodySize: LIMITS.request,
    maxSubscriptions: 4, onerror: () => {} });
  const slots = new Slots(4); const sockets = new Set();
  const server = http.createServer({ maxHeaderSize: 16 * 1024, requestTimeout: 30_000, headersTimeout: 10_000 }, (request, response) => {
    const controller = new AbortController(); const timer = setTimeout(() => { controller.abort(); response.destroy(); }, 30_000);
    response.on('close', () => controller.abort());
    void slots.run(async () => {
      authorize(request, server.address().port, token);
      requireThat(request.method === 'POST', 'method_not_allowed');
      requireThat(request.headers['content-type']?.split(';')[0].trim() === 'application/json', 'invalid_request');
      const { bytes, parsed } = await body(request);
      // This endpoint is stateless request/response. Stdio owns subscriptions,
      // capture grants and optional channel push. No parked HTTP subscription.
      requireThat(!['server/listen', 'subscriptions/listen'].includes(parsed?.method), 'subscription_unavailable');
      const headers = new Headers();
      for (const [name, value] of Object.entries(request.headers)) if (typeof value === 'string' && name !== 'authorization') headers.set(name, value);
      const result = await handler.fetch(new Request(`http://127.0.0.1:${server.address().port}/mcp`,
        { method: 'POST', headers, body: bytes, signal: controller.signal }), { parsedBody: parsed });
      let size = 0; const chunks = [];
      if (result.body) {
        const reader = result.body.getReader();
        try { for (;;) { const next = await reader.read(); if (next.done) break; size += next.value.length; requireThat(size <= LIMITS.response, 'response_too_large'); chunks.push(Buffer.from(next.value)); } }
        finally { await reader.cancel().catch(() => {}); }
      }
      response.writeHead(result.status, { 'content-type': result.headers.get('content-type') ?? 'application/json',
        'cache-control': 'no-store', 'x-content-type-options': 'nosniff' });
      response.end(Buffer.concat(chunks, size));
    }).catch(() => { if (!response.headersSent && !response.destroyed) { response.writeHead(403, { 'content-type': 'application/json', 'cache-control': 'no-store' }); response.end(encoded({ error: 'request_refused' })); } else response.destroy(); })
      .finally(() => clearTimeout(timer));
  });
  server.maxConnections = 8;
  server.on('connection', socket => { sockets.add(socket); socket.setTimeout(35_000, () => socket.destroy()); socket.once('close', () => sockets.delete(socket)); });
  server.on('clientError', (_error, socket) => socket.destroy());
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen({ host: '127.0.0.1', port, exclusive: true }, resolve); });
  return { port: server.address().port, close: async () => { for (const socket of sockets) socket.destroy(); await new Promise(resolve => server.close(resolve)); await handler.close(); } };
}
