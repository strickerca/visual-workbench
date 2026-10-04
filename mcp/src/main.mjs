import { serveStdio } from '@modelcontextprotocol/server/stdio';
import { Writer, Lines, PipeTransport } from './frames.mjs';
import { Owner } from './owner.mjs';
import { Grants } from './grants.mjs';
import { NativeVerifier, Packages } from './packages.mjs';
import { factory } from './server.mjs';
import { listenHttp } from './http.mjs';
import { Slots, requireThat, safeCode, identifier } from './bounds.mjs';
requireThat(process.versions.node === '24.21.0', 'runtime_version');

// The native supervisor is the only producer of this inherited input. Agent
// bytes arrive nested in pipe/message and can never become owner commands.
const writer = new Writer(process.stdout); const owner = new Owner(writer); const grants = new Grants();
const controls = new Slots(4); const toolSlots = new Slots(4); const pipes = new Map();
let packages, http, started = false, stopping = false;
async function shutdown() {
  if (stopping) return; stopping = true; owner.close();
  for (const value of pipes.values()) await value.transport.close();
  await http?.close(); writer.close(); process.stdin.destroy();
}
async function command(message) {
  requireThat(message && typeof message.kind === 'string');
  if (!started) {
    requireThat(message.kind === 'init'); started = true;
    packages = new Packages(new NativeVerifier(message.verifier));
    http = await listenHttp(factory({ packages, owner, grants, principal: 'http:no-capture', slots: toolSlots }), message.token, message.port);
    message.token = undefined;
    await writer.send({ kind: 'ready', port: http.port }); return;
  }
  if (message.kind === 'owner/reply') return owner.reply(message);
  if (message.kind === 'shutdown') return shutdown();
  if (message.kind === 'pipe/open') {
    requireThat(typeof message.connection === 'string' && /^[a-f0-9]{32}$/.test(message.connection) && !pipes.has(message.connection) && pipes.size < 4);
    grants.open(message.connection);
    const transport = new PipeTransport(message.connection, writer, () => { grants.close(message.connection); pipes.delete(message.connection); });
    const make = factory({ packages, owner, grants, principal: message.connection, slots: toolSlots, channel: true });
    const value = { transport, product: null }; pipes.set(message.connection, value);
    await serveStdio(context => { value.product = make(context); return value.product; }, { legacy: 'serve', transport, maxSubscriptions: 4, onerror: () => {} });
    return;
  }
  if (message.kind === 'pipe/message') {
    const pipe = pipes.get(message.connection); requireThat(pipe, 'closed');
    try { pipe.transport.deliver(message.message); } catch { await pipe.transport.close(); } return;
  }
  if (message.kind === 'pipe/closed') { await pipes.get(message.connection)?.transport.close(); return; }
  requireThat(typeof message.id === 'string' && message.id.length <= 64);
  return controls.run(async () => {
    let result;
    switch (message.kind) {
      case 'publish': result = await packages.publish(message.directory, message.manifest_sha256); break;
      case 'unpublish': {
        result = await packages.unpublish(message.package_id, message.target, message.manifest_sha256, undefined, 25_000, () => owner.drain());
        result.owner_callbacks_drained = true; break;
      }
      case 'preview/claude': result = await packages.previewClaude(identifier(message.package_id), message.manifest_sha256); break;
      case 'grant': grants.grant(message.connection, message.selectors, message.lifetime_ms); result = {}; break;
      case 'revoke': grants.revoke(message.connection); result = {}; break;
      case 'push/claude': {
        // Only inherited owner commands reach this branch. Publishing alone
        // never sends; a distinct UI action supplies this exact connection.
        const value = pipes.get(message.connection); requireThat(value?.product, 'closed');
        result = await packages.pushClaude(identifier(message.package_id), message.manifest_sha256, message.displayed, undefined,
          notification => value.product.server.notification(notification)); break;
      }
      default: requireThat(false);
    }
    await writer.send({ kind: 'control/reply', id: message.id, ok: true, result });
  });
}
// Serialize dispatch itself so pipe open/close boundaries cannot be overtaken.
// Long owner operations run independently inside bounded controls; responses
// and cancellation continue to flow while a package is being checked.
let opening = Promise.resolve(); let incoming = 0;
const lines = new Lines(message => {
  requireThat(++incoming <= 16, 'busy');
  if (!started || message.kind === 'pipe/open' || message.kind === 'pipe/message' || message.kind === 'pipe/closed') {
    opening = opening.then(() => command(message)).catch(() => shutdown()).finally(() => incoming--);
  } else void opening.then(() => command(message)).catch(error => writer.send({ kind: 'control/reply', id: message.id, ok: false, code: safeCode(error) }).catch(() => shutdown())).finally(() => incoming--);
}, () => { void shutdown(); });
process.stdin.on('data', chunk => lines.push(chunk));
process.stdin.on('end', () => { lines.end(); void shutdown(); });
process.stdin.on('error', () => { void shutdown(); });
process.stdout.on('error', () => { void shutdown(); });
process.on('uncaughtException', () => { void shutdown().finally(() => { process.exitCode = 1; }); });
process.on('unhandledRejection', () => { void shutdown().finally(() => { process.exitCode = 1; }); });
