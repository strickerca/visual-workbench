import { randomUUID } from 'node:crypto';
import { LIMITS, Refused, requireThat } from './bounds.mjs';
/** Native callbacks retain their slot after caller cancellation until the owner
 * confirms actual settlement. This prevents file retirement racing late work. */
export class Owner {
  #pending = new Map(); #closed = false; #drains = new Set();
  constructor(writer) { this.writer = writer; }
  async call(method, payload, signal, timeout = LIMITS.deadline) {
    requireThat(!this.#closed && this.#pending.size < LIMITS.pending, 'busy');
    if (signal?.aborted) throw new Refused('cancelled'); const id = randomUUID();
    return new Promise((resolve, reject) => {
      let rejected = false;
      const detach = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort); };
      const finish = (error, result) => {
        if (!this.#pending.delete(id)) return; detach();
        if (!rejected) error ? reject(error) : resolve(result);
        if (this.#pending.size === 0) for (const done of [...this.#drains]) done();
      };
      const cancel = code => {
        if (rejected) return; rejected = true; detach(); reject(new Refused(code));
        // A failed cancellation write does not prove the owner is finished.
        void this.writer.send({ kind: 'owner/cancel', id }).catch(() => {});
      };
      const abort = () => cancel('cancelled'); const timer = setTimeout(() => cancel('owner_timeout'), timeout);
      this.#pending.set(id, finish); signal?.addEventListener('abort', abort, { once: true });
      void this.writer.send({ kind: 'owner/request', id, method, payload }).catch(() => cancel('owner_unavailable'));
      if (signal?.aborted) abort();
    });
  }
  reply(message) {
    requireThat(typeof message.id === 'string');
    this.#pending.get(message.id)?.(message.ok === true ? null : new Refused('owner_refused'), message.result);
  }
  async drain(timeout = 25_000) {
    requireThat(!this.#closed, 'closed'); requireThat(Number.isInteger(timeout) && timeout >= 1 && timeout <= 25_000); if (this.#pending.size === 0) return;
    await new Promise((resolve, reject) => {
      const done = () => { clearTimeout(timer); this.#drains.delete(done); this.#closed ? reject(new Refused('closed')) : resolve(); };
      const timer = setTimeout(() => { this.#drains.delete(done); reject(new Refused('owner_busy')); }, timeout);
      this.#drains.add(done);
    });
  }
  close() {
    this.#closed = true;
    for (const finish of [...this.#pending.values()]) finish(new Refused('closed'));
    for (const done of [...this.#drains]) done();
  }
}
