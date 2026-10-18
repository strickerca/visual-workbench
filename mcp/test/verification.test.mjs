import test from 'node:test';
import assert from 'node:assert/strict';
import { crc32, deflateSync } from 'node:zlib';
import { LIMITS, Refused, requireThat } from '../src/bounds.mjs';
import { Owner } from '../src/owner.mjs';
import { registerVerificationTools } from '../src/verification.mjs';

const original = Object.freeze({ package_id: '019a3aaa-0123-789a-8abc-0123456789ab', target: 'generic', manifest_sha256: 'a'.repeat(64) });
const after = Object.freeze({ package_id: '019a3aaa-0123-789a-8abc-0123456789ac', target: 'generic', manifest_sha256: 'b'.repeat(64) });
const receipt = Object.freeze({ receipt_id: '019a3aaa-0123-789a-8abc-0123456789ae', receipt_blake3: 'c'.repeat(64) });
const request = Object.freeze({ ...original, ...receipt, n: 1 });
function chunk(type, body) {
  const bytes = Buffer.alloc(body.length + 12);
  bytes.writeUInt32BE(body.length); bytes.write(type, 4, 4, 'ascii'); body.copy(bytes, 8);
  bytes.writeUInt32BE(crc32(bytes.subarray(4, bytes.length - 4)), bytes.length - 4); return bytes;
}
function image(width = 2, height = 1, { color = 6, depth = 8, interlace = 0, raw, compressed } = {}) {
  const header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4);
  header[8] = depth; header[9] = color; header[12] = interlace;
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[color] ?? 4;
  const pixels = raw ?? Buffer.alloc((1 + Math.ceil(width * channels * depth / 8)) * height);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header),
    ...(color === 3 ? [chunk('PLTE', Buffer.from([0, 0, 0]))] : []),
    chunk('IDAT', compressed ?? deflateSync(pixels)), chunk('IEND', Buffer.alloc(0))]).toString('base64');
}
function entry(value, markers) {
  return Object.freeze({ hash: value.manifest_sha256, manifest: Object.freeze({ package_id: value.package_id,
    compiled_for: Object.freeze({ target: value.target }), markers: Object.freeze(Array.from({ length: markers }, (_, n) => Object.freeze({ n: n + 1 }))) }) });
}
function fixture({ recapture, comparison, owner: injectedOwner, missingAfter = false, afterHash = after.manifest_sha256 } = {}) {
  const calls = [], checks = [], tools = new Map();
  const entries = new Map([[original.package_id, entry(original, 2)],
    ...(missingAfter ? [] : [[after.package_id, entry({ ...after, manifest_sha256: afterHash }, 0)]])]);
  const packages = Object.freeze({ withChecked: async (id, target, signal, action) => {
    checks.push({ id, target, signal });
    if (signal?.aborted) throw new Refused('cancelled');
    const found = entries.get(id); requireThat(found && found.manifest.compiled_for.target === target, 'package_not_published');
    return action(found);
  } });
  const answers = {
    recapture: recapture ?? { original: { ...original }, after: { ...after }, ...receipt, marker_count: 2 },
    comparison: comparison ?? { ...request, status: 'aligned', method: 'semantic', before_png: image(), after_png: image(), width: 2, height: 1 },
  };
  const owner = injectedOwner ?? Object.freeze({ call: async (method, payload, signal) => {
    calls.push({ method, payload, signal });
    return structuredClone(method === 'verification_recapture' ? answers.recapture : answers.comparison);
  } });
  registerVerificationTools((name, properties, required, action, readOnly) => {
    assert.equal(tools.has(name), false); tools.set(name, { properties, required, action, readOnly });
  }, { packages, owner, principal: 'fixture:authorized-owner' });
  return { tools, calls, checks, answers, invoke: (name, args, signal) => tools.get(name).action(args, signal) };
}
const recapture = (f, args = original, signal) => f.invoke('recapture_package', args, signal);
const compare = (f, args = request, signal) => f.invoke('get_marker_comparison', args, signal);
const invalid = pending => assert.rejects(pending, error => error instanceof Refused);

test('registration exposes only the bounded identity-based recapture and comparison tools', () => {
  const f = fixture(); assert.deepEqual([...f.tools.keys()], ['recapture_package', 'get_marker_comparison']);
  assert.deepEqual(f.tools.get('recapture_package').required, Object.keys(original));
  assert.deepEqual(f.tools.get('get_marker_comparison').required, Object.keys(request));
  assert.equal(f.tools.get('recapture_package').readOnly, false); assert.equal(f.tools.get('get_marker_comparison').readOnly, true);
  assert.equal(f.tools.get('get_marker_comparison').properties.n.maximum, 512);
  assert.ok(!JSON.stringify([...f.tools.values()]).includes('selector'));
});

test('recapture checks the original and published after hash, forwarding cancellation and only public identity fields', async () => {
  const f = fixture(), stop = new AbortController(); const result = await recapture(f, original, stop.signal);
  assert.deepEqual(result.structuredContent, { original, after, ...receipt, marker_count: 2 });
  assert.deepEqual(f.checks, [{ id: original.package_id, target: original.target, signal: stop.signal },
    { id: after.package_id, target: after.target, signal: stop.signal }]);
  assert.deepEqual(f.calls, [{ method: 'verification_recapture', payload: { session: 'fixture:authorized-owner', ...original }, signal: stop.signal }]);
  assert.equal(result.content.length, 1); assert.equal(result.content[0].type, 'text');
});

test('canonical UUIDv7 receipts work with the native identity contract', async () => {
  const f = fixture(); const id = '019a3aaa-0123-789a-8abc-0123456789ad';
  f.answers.recapture.receipt_id = id; f.answers.comparison.receipt_id = id;
  assert.equal((await recapture(f)).structuredContent.receipt_id, id);
  assert.equal((await compare(f, { ...request, receipt_id: id })).structuredContent.receipt_id, id);
  await invalid(recapture(f, { ...original, package_id: 'e2438a7a-4efe-4a82-ae1b-23368589034e' }));
});

test('UUIDv4 receipt IDs refuse in the public schema, requests and both owner responses', async () => {
  const v4 = 'e2438a7a-4efe-4a82-ae1b-23368589034e';
  const input = fixture();
  assert.equal(new RegExp(input.tools.get('get_marker_comparison').properties.receipt_id.pattern).test(v4), false);
  await invalid(compare(input, { ...request, receipt_id: v4 })); assert.equal(input.calls.length, 0);
  const recaptured = fixture(); recaptured.answers.recapture.receipt_id = v4; await invalid(recapture(recaptured));
  const compared = fixture(); compared.answers.comparison.receipt_id = v4; await invalid(compare(compared));
});

test('aligned comparisons emit ordered image content with public metadata and no structured base64', async () => {
  for (const method of ['semantic', 'registration']) {
    const f = fixture(); f.answers.comparison.method = method; const stop = new AbortController(); const result = await compare(f, request, stop.signal);
    assert.deepEqual(result.structuredContent, { ...request, status: 'aligned', method, width: 2, height: 1 });
    assert.deepEqual(result.content.slice(1), [{ type: 'image', mimeType: 'image/png', data: f.answers.comparison.before_png },
      { type: 'image', mimeType: 'image/png', data: f.answers.comparison.after_png }]);
    assert.equal(JSON.stringify(result.structuredContent).includes(f.answers.comparison.before_png), false);
    assert.deepEqual(f.calls, [{ method: 'verification_compare', payload: { session: 'fixture:authorized-owner', ...request }, signal: stop.signal }]);
  }
});

test('unresolved comparisons return text and bounded geometry without images or success inference', async () => {
  for (const [width, height] of [[0, 0], [512, 512]]) {
    const f = fixture({ comparison: { ...request, status: 'unresolved', method: null, before_png: null, after_png: null, width, height } });
    const result = await compare(f); assert.deepEqual(result.structuredContent, { ...request, status: 'unresolved', method: null, width, height });
    assert.equal(result.content.length, 1); assert.match(result.content[0].text, /unresolved/);
  }
});

test('invalid, extra and selector-bearing requests refuse before calling the owner', async () => {
  const f = fixture();
  for (const value of [null, [], { ...original, selector: 'unapproved' }, { ...original, target: 'other' },
    { ...original, manifest_sha256: 'A'.repeat(64) }, { ...original, package_id: '../private' }]) await invalid(recapture(f, value));
  for (const value of [null, { ...request, note: 'private' }, { ...request, n: 0 }, { ...request, n: 3 },
    { ...request, n: 513 }, { ...request, n: 1.5 }, { ...request, receipt_id: 'invalid' },
    { ...request, receipt_blake3: 'z'.repeat(64) }, { ...request, manifest_sha256: 'd'.repeat(64) }]) await invalid(compare(f, value));
  assert.equal(f.calls.length, 0);
});

test('a changed original hash refuses recapture before owner work', async () => {
  const f = fixture(); await invalid(recapture(f, { ...original, manifest_sha256: 'd'.repeat(64) })); assert.equal(f.calls.length, 0);
});

test('unknown owner fields, mismatched identities, receipts and marker counts refuse recapture', async () => {
  const make = () => ({ original: { ...original }, after: { ...after }, ...receipt, marker_count: 2 });
  const variants = [null, [], { ...make(), note: 'private' }, { ...make(), selector: 'private' },
    { ...make(), original: { ...original, note: 'private' } }, { ...make(), after: { ...after, verdict: 'pass' } },
    { ...make(), original: { ...original, manifest_sha256: 'd'.repeat(64) } },
    { ...make(), after: { ...original } }, { ...make(), after: { ...after, target: 'claude' } },
    { ...make(), after: { ...after, manifest_sha256: 'bad' } }, { ...make(), receipt_id: 'bad' },
    { ...make(), receipt_blake3: 'bad' }, { ...make(), marker_count: 1 }, { ...make(), marker_count: '2' },
    { ...make(), marker_count: 513 }];
  for (const value of variants) {
    const f = fixture(); f.answers.recapture = value; await invalid(recapture(f));
  }
});

test('recapture refuses an unpublished after or a changed published after hash', async () => {
  await invalid(recapture(fixture({ missingAfter: true })));
  await invalid(recapture(fixture({ afterHash: 'd'.repeat(64) })));
});

test('comparison refuses unknown fields and all identity/receipt/marker mismatches', async () => {
  const f = fixture(), valid = f.answers.comparison;
  for (const wrong of [null, [], { ...valid, note: 'private' }, { ...valid, verdict: 'pass' },
    { ...valid, selector: 'private' }, { ...valid, package_id: after.package_id }, { ...valid, target: 'claude' },
    { ...valid, manifest_sha256: after.manifest_sha256 }, { ...valid, receipt_id: after.package_id },
    { ...valid, receipt_blake3: 'd'.repeat(64) }, { ...valid, n: 2 }, { ...valid, status: 'pass' }]) {
    f.answers.comparison = wrong; await invalid(compare(f));
  }
});

test('comparison rejects absent/unpaired crops, invalid methods and oversized or fractional geometry', async () => {
  const f = fixture(), valid = f.answers.comparison;
  for (const patch of [{ before_png: null }, { after_png: null }, { method: null }, { method: 'dimensions' },
    { width: 0, height: 0 }, { width: 0 }, { width: 513 }, { height: 513 }, { width: -1 },
    { width: 1.5 }, { height: Number.NaN }, { status: 'unresolved' },
    { status: 'unresolved', before_png: null, after_png: null },
    { status: 'unresolved', method: null, before_png: null },
    { status: 'unresolved', method: null, before_png: null, after_png: null, width: 0 }]) {
    f.answers.comparison = { ...valid, ...patch }; await invalid(compare(f));
  }
});

test('standard RGB, grayscale, palette and RGBA PNGs remain compatible at the crop limit', async () => {
  for (const [color, depth] of [[0, 1], [0, 16], [2, 8], [2, 16], [3, 8], [4, 8], [6, 8], [6, 16]]) {
    const f = fixture(), data = image(512, 512, { color, depth });
    Object.assign(f.answers.comparison, { width: 512, height: 512, before_png: data, after_png: data });
    assert.equal((await compare(f)).content.length, 3);
  }
  // A 1x1 Adam7 image has a single nonempty pass, with one RGBA scanline.
  const f = fixture(), data = image(1, 1, { interlace: 1 });
  Object.assign(f.answers.comparison, { width: 1, height: 1, before_png: data, after_png: data });
  assert.equal((await compare(f)).content.length, 3);
});

test('PNG validation rejects malformed base64, mismatched dimensions, truncation, CRC and compressed-data failures', async () => {
  const f = fixture(), valid = f.answers.comparison, bytes = Buffer.from(valid.before_png, 'base64');
  const badCrc = Buffer.from(bytes); badCrc[29] ^= 1;
  const signatureOnly = bytes.subarray(0, 8).toString('base64');
  const badFilter = Buffer.alloc(9); badFilter[0] = 5;
  const rawOversize = Buffer.alloc(10);
  for (const data of ['', '%%%%', valid.before_png + '\n', signatureOnly, bytes.subarray(0, -1).toString('base64'),
    badCrc.toString('base64'), image(3, 1), image(2, 1, { depth: 3 }), image(2, 1, { interlace: 2 }),
    image(2, 1, { compressed: Buffer.from('invalid zlib') }), image(2, 1, { raw: badFilter }),
    image(2, 1, { raw: rawOversize }), image(2, 1, { raw: Buffer.alloc(8) }),
    image(2, 1, { compressed: Buffer.concat([deflateSync(Buffer.alloc(9)), Buffer.from([0])]) })]) {
    f.answers.comparison = { ...valid, before_png: data }; await invalid(compare(f));
  }
});

test('encoded PNG bytes are capped before decompression or public return', async () => {
  const f = fixture(), data = Buffer.alloc(LIMITS.image + 1).toString('base64');
  f.answers.comparison.before_png = data; await invalid(compare(f));
});

test('owner-sorted JSON field order is accepted while missing mandatory fields refuse', async () => {
  const f = fixture();
  const sorted = value => value && typeof value === 'object' && !Array.isArray(value)
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sorted(value[key])])) : value;
  f.answers.recapture = sorted(f.answers.recapture); f.answers.comparison = sorted(f.answers.comparison);
  await recapture(f); await compare(f);
  delete f.answers.recapture.marker_count; await invalid(recapture(f));
  delete f.answers.comparison.before_png; await invalid(compare(f));
});

test('already-cancelled calls never contact the owner', async () => {
  const f = fixture(), stop = new AbortController(); stop.abort();
  await assert.rejects(recapture(f, original, stop.signal), error => error.code === 'cancelled');
  await assert.rejects(compare(f, request, stop.signal), error => error.code === 'cancelled');
  assert.equal(f.calls.length, 0);
});

test('a native callback that ignores a delivered abort cannot expose a late success', async () => {
  const stop = new AbortController(); let resolve; const settled = new Promise(done => { resolve = done; });
  const owner = Object.freeze({ call: async (_method, _payload, signal) => {
    assert.equal(signal, stop.signal); stop.abort(); return settled;
  } });
  const f = fixture({ owner }); const refused = assert.rejects(recapture(f, original, stop.signal), error => error.code === 'cancelled');
  resolve(f.answers.recapture); await refused; assert.equal(f.checks.length, 1);
});

test('real Owner cancellation is forwarded and retains native admission until the exact late reply', async () => {
  const messages = []; let requested; const received = new Promise(done => { requested = done; });
  const owner = new Owner({ send: async message => { messages.push(message); if (message.kind === 'owner/request') requested(); } });
  const f = fixture({ owner }), stop = new AbortController();
  try {
    const pending = compare(f, request, stop.signal); await received;
    const refusal = assert.rejects(pending, error => error.code === 'cancelled'); stop.abort(); await refusal;
    assert.equal(messages[0].method, 'verification_compare');
    assert.deepEqual(messages[1], { kind: 'owner/cancel', id: messages[0].id });
    await assert.rejects(owner.drain(1), error => error.code === 'owner_busy');
    owner.reply({ id: messages[0].id, ok: true, result: f.answers.comparison }); await owner.drain(10);
  } finally { owner.close(); }
});
