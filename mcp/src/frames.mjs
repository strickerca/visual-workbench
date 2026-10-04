import { LIMITS, Refused, encoded, json, requireThat } from './bounds.mjs';
/** Bounded newline framing. Oversize frames fail before Buffer.concat; a peer
 * cannot leave an unbounded unterminated line in readline's internal buffer. */
export class Lines {
  #parts = []; #size = 0; #closed = false;
  constructor(deliver, fail, cap = LIMITS.wire) { this.deliver = deliver; this.fail = fail; this.cap = cap; }
  push(chunk) {
    if (this.#closed) return;
    try {
      let start = 0;
      for (let i = 0; i < chunk.length; i++) if (chunk[i] === 10) {
        this.#append(chunk.subarray(start, i));
        const frame = Buffer.concat(this.#parts, this.#size); this.#parts = []; this.#size = 0;
        requireThat(frame.length > 0); this.deliver(json(frame, this.cap)); start = i + 1;
      }
      this.#append(chunk.subarray(start));
    } catch (error) { this.#closed = true; this.#parts = []; this.#size = 0; this.fail(error); }
  }
  #append(part) {
    requireThat(this.#size + part.length <= this.cap, 'frame_too_large');
    if (part.length) { this.#parts.push(Buffer.from(part)); this.#size += part.length; }
    // Avoid one array element for every one-byte network fragment.
    if (this.#parts.length >= 256) this.#parts = [Buffer.concat(this.#parts, this.#size)];
  }
  end() { if (this.#size) this.fail(new Refused('truncated_frame')); this.#closed = true; this.#parts = []; }
}
export class Writer {
  #queued = 0; #tail = Promise.resolve(); #closed = false;
  constructor(stream, cap = LIMITS.wire) { this.stream = stream; this.cap = cap; }
  send(value) {
    requireThat(!this.#closed, 'closed'); const bytes = encoded(value, this.cap);
    requireThat(this.#queued + bytes.length <= this.cap * 2, 'backpressure'); this.#queued += bytes.length;
    const task = this.#tail.then(() => new Promise((resolve, reject) => {
      let done = false;
      const timer = setTimeout(() => { if (!done) { done = true; this.#closed = true; reject(new Refused('write_timeout')); this.stream.destroy?.(); } }, 15_000);
      this.stream.write(Buffer.concat([bytes, Buffer.from('\n')]), error => {
        if (done) return; done = true; clearTimeout(timer); error ? reject(new Refused('write_failed')) : resolve();
      });
    })).finally(() => { this.#queued -= bytes.length; });
    this.#tail = task.catch(() => { this.#closed = true; }); return task;
  }
  close() { this.#closed = true; }
}
/** SDK Transport over a per-user authenticated pipe relayed by the desktop.
 * The connection identity is assigned by the listener, never by MCP payloads. */
export class PipeTransport {
  #closed = false; #pending = new Map();
  constructor(id, writer, ended) { this.id = id; this.writer = writer; this.ended = ended; }
  async start() {}
  deliver(message) {
    requireThat(!this.#closed, 'closed'); encoded(message, LIMITS.request);
    if (message?.method && message.id !== undefined) {
      requireThat((typeof message.id === 'string' && message.id.length <= 128) || Number.isSafeInteger(message.id));
      requireThat(!this.#pending.has(message.id) && this.#pending.size < LIMITS.pending, 'busy');
      // A cancelled/parked request may never produce a terminal response. Expire
      // the entire connection, never release a live operation's admission slot.
      this.#pending.set(message.id, setTimeout(() => { void this.close(); }, 90_000));
    }
    this.onmessage?.(message);
  }
  async send(message) {
    if (this.#closed) throw new Refused('closed');
    if (!message.method && message.id !== undefined) { clearTimeout(this.#pending.get(message.id)); this.#pending.delete(message.id); }
    await this.writer.send({ kind: 'pipe/send', connection: this.id, message });
  }
  async close() {
    if (this.#closed) return; this.#closed = true; for (const timer of this.#pending.values()) clearTimeout(timer); this.#pending.clear();
    await this.writer.send({ kind: 'pipe/close', connection: this.id }).catch(() => {});
    this.ended(); this.onclose?.();
  }
}
