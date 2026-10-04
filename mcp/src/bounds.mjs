export const LIMITS = Object.freeze({ request: 6 * 1024 * 1024, response: 16 * 1024 * 1024,
  wire: 17 * 1024 * 1024, pending: 8, connections: 4, packages: 64, deadline: 30_000,
  text: 32 * 1024, image: 4 * 1024 * 1024, result: 4 * 1024 * 1024 });
export class Refused extends Error {
  constructor(code) { super(code); this.code = code; }
}
export function requireThat(ok, code = 'invalid_request') { if (!ok) throw new Refused(code); }
export const ID_PATTERN = '^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$';
export function identifier(id) { requireThat(typeof id === 'string' && new RegExp(ID_PATTERN).test(id)); return id; }
export function hex(hash) { requireThat(typeof hash === 'string' && /^[0-9a-f]{64}$/.test(hash)); return hash; }
export function boundedString(text, cap = LIMITS.text) {
  requireThat(typeof text === 'string' && Buffer.byteLength(text) <= cap && !text.includes('\0'));
  return text;
}
export function json(bytes, cap = LIMITS.request) {
  requireThat(bytes.length <= cap, 'request_too_large');
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  // Bound syntactic containers before JSON.parse's object allocation. Strings
  // are byte-capped above; escape handling prevents quoted delimiters counting.
  let depth = 0, nodes = 0, quoted = false, escaped = false;
  for (const ch of text) {
    if (quoted) { if (escaped) escaped = false; else if (ch === '\\') escaped = true; else if (ch === '"') quoted = false; continue; }
    if (ch === '"') { quoted = true; nodes++; }
    else if (ch === '{' || ch === '[') { depth++; nodes++; }
    else if (ch === '}' || ch === ']') depth--;
    else if (ch === ',') nodes++;
    requireThat(depth <= 32 && nodes <= 32768, 'request_too_complex');
  }
  try {
    const result = JSON.parse(text);
    const pending = [result];
    while (pending.length) {
      const value = pending.pop();
      if (typeof value === 'number') requireThat(Number.isFinite(value) && (!Number.isInteger(value) || Number.isSafeInteger(value)), 'numeric_range');
      else if (value && typeof value === 'object') for (const item of Object.values(value)) pending.push(item);
    }
    return result;
  } catch (error) { if (error instanceof Refused) throw error; throw new Refused('invalid_json'); }
}
export function encoded(value, cap = LIMITS.response) {
  const result = Buffer.from(JSON.stringify(value));
  requireThat(result.length <= cap, 'response_too_large'); return result;
}
export class Slots {
  #active = 0;
  constructor(cap) { this.cap = cap; }
  async run(action) {
    requireThat(this.#active < this.cap, 'busy'); this.#active++;
    try { return await action(); } finally { this.#active--; }
  }
  get active() { return this.#active; }
}
export function safeCode(error) { return error instanceof Refused ? error.code : 'operation_failed'; }
