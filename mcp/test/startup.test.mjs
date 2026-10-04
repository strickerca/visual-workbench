import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';

// Exercise the actual SDK entrypoint: factory-only tests miss failures before
// JS dispatch, including Windows Node crypto initialization without SystemRoot.
// The native launcher independently obtains SystemRoot from the OS; this test
// uses the already validated test process value and inherits nothing else.
async function startAndStop(endInput) {
  const env = process.platform === 'win32' ? { SystemRoot: process.env.SystemRoot } : {};
  if (process.platform === 'win32') assert.ok(env.SystemRoot);
  const child = spawn(process.execPath, [fileURLToPath(new URL('../src/main.mjs', import.meta.url))],
    { env, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  let failure, size = 0, stderrSize = 0, output = '';
  let readyResolve, readyReject;
  const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
  const stop = code => { failure ??= new Error(code); child.kill(); };
  const closed = new Promise(resolve => {
    child.once('error', () => { stop('spawn_failed'); readyReject(failure); });
    child.once('close', (code, signal) => { readyReject(failure ?? new Error('closed_before_ready')); resolve({ code, signal }); });
  });
  const timer = setTimeout(() => stop('startup_or_shutdown_timeout'), 10_000);
  child.stdin.on('error', () => stop('input_failed'));
  child.stderr.on('data', chunk => { stderrSize += chunk.length; if (stderrSize > 8192) stop('stderr_bound'); });
  child.stdout.on('data', chunk => {
    size += chunk.length;
    if (size > 4096) return stop('stdout_bound');
    output += chunk.toString('utf8');
    if (!output.includes('\n')) return;
    try {
      const message = JSON.parse(output.trim());
      assert.deepEqual(Object.keys(message).sort(), ['kind', 'port']);
      assert.equal(message.kind, 'ready');
      assert.ok(Number.isInteger(message.port) && message.port > 0 && message.port <= 65535);
      readyResolve();
    } catch { stop('ready_contract'); }
  });
  try {
    child.stdin.write(JSON.stringify({ kind: 'init', token: randomBytes(32).toString('hex'), port: 0,
      verifier: fileURLToPath(new URL('unexecuted-verifier-fixture.exe', import.meta.url)) }) + '\n');
    await ready;
    if (endInput) child.stdin.end();
    else child.stdin.write('{"kind":"shutdown"}\n');
    const result = await closed;
    if (failure) throw failure;
    assert.deepEqual(result, { code: 0, signal: null });
    assert.equal(output.trim().split('\n').length, 1);
  } finally {
    child.kill();
    await closed;
    clearTimeout(timer);
  }
}

test('real SDK entrypoint initializes with a minimal environment and joins explicit shutdown',
  { timeout: 15_000 }, () => startAndStop(false));
test('real SDK entrypoint joins shutdown when its owner input closes',
  { timeout: 15_000 }, () => startAndStop(true));
