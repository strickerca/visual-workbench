import { requireThat, Refused } from './bounds.mjs';
/** Owner-channel only. MCP tools cannot mint, widen or renew grants. Each grant
 * is scoped to an OS-authenticated connection and exact owner selector IDs. */
export class Grants {
  #sessions = new Map();
  constructor(clock = () => performance.now()) { this.clock = clock; }
  open(id) { requireThat(!this.#sessions.has(id) && this.#sessions.size < 8, 'busy'); this.#sessions.set(id, null); }
  close(id) { this.#sessions.get(id)?.abort.abort(); this.#sessions.delete(id); }
  grant(id, selectors, lifetimeMs) {
    requireThat(this.#sessions.has(id) && Array.isArray(selectors) && selectors.length >= 1 && selectors.length <= 16);
    requireThat(selectors.every(s => typeof s === 'string' && /^[a-zA-Z0-9_-]{1,64}$/.test(s)));
    requireThat(Number.isInteger(lifetimeMs) && lifetimeMs >= 1 && lifetimeMs <= 600_000);
    this.#sessions.get(id)?.abort.abort();
    this.#sessions.set(id, { selectors: new Set(selectors), expires: this.clock() + lifetimeMs, abort: new AbortController() });
  }
  revoke(id) { this.#sessions.get(id)?.abort.abort(); if (this.#sessions.has(id)) this.#sessions.set(id, null); }
  admit(id, selector) {
    const value = this.#sessions.get(id);
    if (!value || value.expires <= this.clock() || !value.selectors.has(selector)) throw new Refused('capture_grant_required');
    const check = () => { const current = this.#sessions.get(id); requireThat(current === value && value.expires > this.clock(), 'capture_grant_expired'); };
    check.signal = AbortSignal.any([value.abort.signal, AbortSignal.timeout(Math.max(1, Math.ceil(value.expires - this.clock())))]);
    return check;
  }
}
