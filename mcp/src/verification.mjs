import { crc32, inflateSync } from 'node:zlib';
import { ID_PATTERN, LIMITS, Refused, encoded, hex, identifier, requireThat } from './bounds.mjs';

const targets = ['claude', 'openai', 'gemini', 'generic'];
const identityFields = ['package_id', 'target', 'manifest_sha256'];
const comparisonFields = [...identityFields, 'receipt_id', 'receipt_blake3', 'n'];
const idSchema = { type: 'string', pattern: ID_PATTERN };
// Native inbox receipts and stored review pairs use UUIDv7 vw_model::Id values.
const receiptPattern = ID_PATTERN;
const receiptSchema = { type: 'string', pattern: receiptPattern };
const hashSchema = { type: 'string', pattern: '^[0-9a-f]{64}$' };
const identitySchema = { package_id: idSchema, target: { enum: targets }, manifest_sha256: hashSchema };
const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

function fields(value, names) {
  requireThat(value !== null && typeof value === 'object' && !Array.isArray(value));
  const keys = Reflect.ownKeys(value);
  requireThat(keys.length === names.length && names.every(name => Object.hasOwn(value, name)));
}
function identity(value) {
  fields(value, identityFields);
  identifier(value.package_id); hex(value.manifest_sha256); requireThat(targets.includes(value.target));
  return { package_id: value.package_id, target: value.target, manifest_sha256: value.manifest_sha256 };
}
function equalIdentity(actual, expected) {
  requireThat(identityFields.every(name => actual[name] === expected[name]), 'package_changed');
}
function checkedIdentity(entry, expected) {
  equalIdentity({ package_id: entry.manifest.package_id, target: entry.manifest.compiled_for.target, manifest_sha256: entry.hash }, expected);
}
function markerCount(entry) {
  requireThat(Array.isArray(entry.manifest.markers) && entry.manifest.markers.length <= 512);
  return entry.manifest.markers.length;
}
function cancelled(signal) { if (signal?.aborted) throw new Refused('cancelled'); }
function receiptId(value) { identifier(value); }

/** Validate bounded PNG framing, checksums, geometry and decompressed scanlines.
 * The checked native owner supplies the crop/correspondence; parsing a PNG here
 * never establishes alignment or an owner assessment. */
function png(data, width, height) {
  const maximum = Math.ceil(LIMITS.image / 3) * 4;
  requireThat(typeof data === 'string' && data.length > 0 && data.length <= maximum && data.length % 4 === 0);
  const bytes = Buffer.from(data, 'base64');
  requireThat(bytes.length <= LIMITS.image && bytes.toString('base64') === data && bytes.subarray(0, 8).equals(pngSignature));
  let at = 8, count = 0, header = false, palette = false, dataEnded = false, ended = false;
  let depth, color, interlace;
  const compressed = [];
  while (at < bytes.length) {
    requireThat(++count <= 4096 && at + 12 <= bytes.length);
    const length = bytes.readUInt32BE(at), end = at + 12 + length;
    requireThat(end <= bytes.length);
    const type = bytes.toString('latin1', at + 4, at + 8);
    requireThat(/^[A-Za-z]{2}[A-Z][A-Za-z]$/.test(type));
    requireThat(crc32(bytes.subarray(at + 4, end - 4)) === bytes.readUInt32BE(end - 4));
    const body = bytes.subarray(at + 8, end - 4);
    if (type === 'IHDR') {
      requireThat(!header && at === 8 && length === 13);
      requireThat(body.readUInt32BE(0) === width && body.readUInt32BE(4) === height);
      depth = body[8]; color = body[9]; interlace = body[12];
      const depths = { 0: [1, 2, 4, 8, 16], 2: [8, 16], 3: [1, 2, 4, 8], 4: [8, 16], 6: [8, 16] };
      requireThat(depths[color]?.includes(depth) && body[10] === 0 && body[11] === 0 && interlace <= 1);
      header = true;
    } else {
      requireThat(header);
      if (type === 'PLTE') {
        requireThat(!palette && compressed.length === 0 && color !== 0 && color !== 4 && length >= 3 && length <= 768 && length % 3 === 0);
        requireThat(color !== 3 || length / 3 <= 2 ** depth); palette = true;
      } else if (type === 'IDAT') {
        requireThat(!dataEnded && (color !== 3 || palette)); compressed.push(body);
      } else if (type === 'IEND') {
        requireThat(compressed.length > 0 && length === 0 && end === bytes.length); ended = true;
      } else {
        // Unknown critical chunks and animations cannot describe a single crop.
        requireThat(type[0] === type[0].toLowerCase() && !['acTL', 'fcTL', 'fdAT'].includes(type));
        if (compressed.length) dataEnded = true;
      }
    }
    at = end;
  }
  requireThat(ended);
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[color];
  const passes = interlace === 0 ? [[0, 0, 1, 1]] : [[0, 0, 8, 8], [4, 0, 8, 8], [0, 4, 4, 8], [2, 0, 4, 4], [0, 2, 2, 4], [1, 0, 2, 2], [0, 1, 1, 2]];
  const rows = [];
  for (const [x, y, dx, dy] of passes) {
    const columns = Math.max(0, Math.ceil((width - x) / dx)), lines = Math.max(0, Math.ceil((height - y) / dy));
    if (columns && lines) for (let n = 0; n < lines; n++) rows.push(1 + Math.ceil(columns * channels * depth / 8));
  }
  const expected = rows.reduce((total, length) => total + length, 0), input = Buffer.concat(compressed);
  let decoded;
  try { decoded = inflateSync(input, { maxOutputLength: expected, info: true }); }
  catch { throw new Refused('invalid_request'); }
  requireThat(decoded.buffer.length === expected && decoded.engine.bytesWritten === input.length);
  let row = 0;
  for (const length of rows) { requireThat(decoded.buffer[row] <= 4); row += length; }
  return { type: 'image', mimeType: 'image/png', data };
}

/** Public immutable identities only. The Desktop owner resolves retained native
 * selectors and rechecks its exact grant; no agent field can supply either. */
export function registerVerificationTools(tool, { packages, owner, principal }) {
  tool('recapture_package', identitySchema, identityFields, async (args, signal) => {
    const original = identity(args);
    return packages.withChecked(original.package_id, original.target, signal, async entry => {
      checkedIdentity(entry, original); const markers = markerCount(entry); cancelled(signal);
      const result = await owner.call('verification_recapture', { session: principal, ...original }, signal);
      cancelled(signal);
      fields(result, ['original', 'after', 'receipt_id', 'receipt_blake3', 'marker_count']);
      const before = identity(result.original), after = identity(result.after);
      equalIdentity(before, original);
      requireThat(after.package_id !== original.package_id && after.target === original.target, 'package_changed');
      receiptId(result.receipt_id); hex(result.receipt_blake3);
      requireThat(Number.isInteger(result.marker_count) && result.marker_count === markers);
      return packages.withChecked(after.package_id, after.target, signal, async published => {
        checkedIdentity(published, after); cancelled(signal);
        const response = {
          content: [{ type: 'text', text: 'Recapture published. Marker correspondence and owner review remain separate.' }],
          structuredContent: { original: before, after, receipt_id: result.receipt_id, receipt_blake3: result.receipt_blake3, marker_count: markers },
        };
        encoded(response); return response;
      });
    });
  }, false);

  tool('get_marker_comparison', { ...identitySchema, receipt_id: receiptSchema, receipt_blake3: hashSchema,
    n: { type: 'integer', minimum: 1, maximum: 512 } }, comparisonFields, async (args, signal) => {
    fields(args, comparisonFields);
    const original = identity(Object.fromEntries(identityFields.map(name => [name, args[name]])));
    receiptId(args.receipt_id); hex(args.receipt_blake3);
    requireThat(Number.isInteger(args.n) && args.n >= 1 && args.n <= 512);
    return packages.withChecked(original.package_id, original.target, signal, async entry => {
      checkedIdentity(entry, original); requireThat(args.n <= markerCount(entry)); cancelled(signal);
      const receipt = { receipt_id: args.receipt_id, receipt_blake3: args.receipt_blake3, n: args.n };
      const result = await owner.call('verification_compare', { session: principal, ...original, ...receipt }, signal);
      cancelled(signal);
      fields(result, [...comparisonFields, 'status', 'method', 'before_png', 'after_png', 'width', 'height']);
      const returned = identity(Object.fromEntries(identityFields.map(name => [name, result[name]])));
      equalIdentity(returned, original); receiptId(result.receipt_id); hex(result.receipt_blake3);
      requireThat(Object.keys(receipt).every(name => result[name] === receipt[name]), 'comparison_changed');
      requireThat(['aligned', 'unresolved'].includes(result.status));
      requireThat(Number.isInteger(result.width) && result.width >= 0 && result.width <= 512
        && Number.isInteger(result.height) && result.height >= 0 && result.height <= 512
        && (result.width === 0) === (result.height === 0));
      let content;
      if (result.status === 'aligned') {
        requireThat(['semantic', 'registration'].includes(result.method) && result.width > 0 && result.height > 0);
        content = [{ type: 'text', text: 'Aligned marker crops: before, then after. This is correspondence, not an owner verdict.' },
          png(result.before_png, result.width, result.height), png(result.after_png, result.width, result.height)];
      } else {
        requireThat(result.method === null && result.before_png === null && result.after_png === null);
        content = [{ type: 'text', text: 'Marker correspondence is unresolved; no aligned comparison is available.' }];
      }
      const response = { content, structuredContent: { ...original, ...receipt, status: result.status,
        method: result.method, width: result.width, height: result.height } };
      encoded(response); return response;
    });
  }, true);
}
