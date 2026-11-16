import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';

// Nothing an agent connection writes may end the shared server. These drive
// the real entrypoint over its inherited owner channel the way the native
// supervisor does: each agent line travels as one opaque string.
const hello = id => JSON.stringify({ jsonrpc: '2.0', id, method: 'initialize',
  params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'fixture', version: '0' } } });
async function session(body) {
  const env = process.platform === 'win32' ? { SystemRoot: process.env.SystemRoot } : {};
  const child = spawn(process.execPath, [fileURLToPath(new URL('../src/main.mjs', import.meta.url))],
    { env, windowsHide: true, stdio: ['pipe', 'pipe', 'ignore'] });
  const seen = [], waiters = new Set(); let text = '', exited = false;
  const closed = new Promise(resolve => child.once('close', (code, signal) => { exited = true; for (const wake of waiters) wake(); resolve({ code, signal }); }));
  child.stdout.on('data', chunk => {
    text += chunk.toString('utf8'); const parts = text.split('\n'); text = parts.pop();
    for (const part of parts) seen.push(JSON.parse(part));
    for (const wake of waiters) wake();
  });
  const send = (...frames) => child.stdin.write(frames.map(frame => JSON.stringify(frame) + '\n').join(''));
  const until = async (match, label) => {
    for (;;) {
      const at = seen.findIndex(match); if (at >= 0) return seen.splice(at, 1)[0];
      assert.equal(exited, false, `server exited while waiting for ${label}`);
      await new Promise(resolve => { const wake = () => { waiters.delete(wake); resolve(); }; waiters.add(wake); });
    }
  };
  const open = async connection => {
    send({ kind: 'pipe/open', connection }, { kind: 'pipe/message', connection, message: hello(1) });
    const reply = await until(m => m.kind === 'pipe/send' && m.connection === connection, 'initialize');
    assert.equal(reply.message.id, 1); assert.equal(reply.message.result.serverInfo.name, 'visual-workbench');
  };
  const timer = setTimeout(() => child.kill(), 20_000);
  try {
    send({ kind: 'init', token: randomBytes(32).toString('hex'), port: 0, verifier: fileURLToPath(new URL('unexecuted-verifier-fixture.exe', import.meta.url)) });
    await until(m => m.kind === 'ready', 'ready');
    await body({ send, until, open });
    send({ kind: 'shutdown' }); assert.deepEqual(await closed, { code: 0, signal: null });
  } finally { clearTimeout(timer); child.kill(); await closed; }
}
const a = 'a'.repeat(32), b = 'b'.repeat(32), c = 'c'.repeat(32);

test('an agent line the owner frame could not carry ends only that connection', { timeout: 30_000 }, () => session(async ({ send, until, open }) => {
  await open(a);
  for (const [connection, line] of [
    [b, '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_packages","arguments":{"limit":18446744073709551615}}}'],
    [c, `{"jsonrpc":"2.0","method":"notifications/x","params":${'['.repeat(32)}${']'.repeat(32)}}`],
    [b, '{"jsonrpc":"2.0","id":3,"method":"ping","params":{"n":1e22}}'],
    [c, 'not json'],
  ]) {
    await open(connection); send({ kind: 'pipe/message', connection, message: line });
    await until(m => m.kind === 'pipe/close' && m.connection === connection, 'the refused connection to close');
    send({ kind: 'pipe/closed', connection });
  }
  send({ kind: 'pipe/message', connection: a, message: '{"jsonrpc":"2.0","id":9,"method":"ping"}' });
  assert.deepEqual((await until(m => m.kind === 'pipe/send' && m.connection === a, 'the surviving connection')).message, { jsonrpc: '2.0', id: 9, result: {} });
}));

test('lines already forwarded for a connection this side closed are dropped', { timeout: 30_000 }, () => session(async ({ send, until, open }) => {
  await open(a); await open(b);
  send({ kind: 'pipe/message', connection: b, message: '{"jsonrpc":"2.0","id":null,"method":"ping"}' },
    { kind: 'pipe/message', connection: b, message: '{"jsonrpc":"2.0","id":4,"method":"ping"}' },
    { kind: 'pipe/message', connection: 'd'.repeat(32), message: '{"jsonrpc":"2.0","id":5,"method":"ping"}' });
  await until(m => m.kind === 'pipe/close' && m.connection === b, 'the refused connection to close');
  send({ kind: 'pipe/message', connection: a, message: '{"jsonrpc":"2.0","id":9,"method":"ping"}' });
  assert.equal((await until(m => m.kind === 'pipe/send' && m.connection === a, 'the surviving connection')).message.id, 9);
}));

test('a burst from one agent sheds that connection and leaves owner commands admitted', { timeout: 30_000 }, () => session(async ({ send, until, open }) => {
  await open(a); await open(b);
  send(...Array.from({ length: 64 }, (_, n) => ({ kind: 'pipe/message', connection: b, message: `{"jsonrpc":"2.0","id":${n + 10},"method":"ping"}` })),
    { kind: 'revoke', id: 'after-burst', connection: a });
  await until(m => m.kind === 'pipe/close' && m.connection === b, 'the bursting connection to close');
  assert.equal((await until(m => m.kind === 'control/reply' && m.id === 'after-burst', 'the owner command')).ok, true);
  send({ kind: 'pipe/message', connection: a, message: '{"jsonrpc":"2.0","id":9,"method":"ping"}' });
  assert.equal((await until(m => m.kind === 'pipe/send' && m.connection === a, 'the surviving connection')).message.id, 9);
}));
