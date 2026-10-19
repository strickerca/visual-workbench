package com.visualworkbench.shared

/** One endpoint-session pool shared with native ring/codec/cache owners. The
 * implementation must wrap that supplied pool, never construct a per-call pool.
 * Reserve before allocating/copying foreign pixels or creating a drawing resource.
 * Platform stride/surface limits still apply in addition to this byte admission. */
public expect fun sessionResources(): SessionResources

public interface SessionResources {
    public suspend fun reserveForeign(bytes: ULong): SessionResourceCharge
}
/** Non-copyable by contract. Retain beside the actual foreign resource and close
 * only after that resource and its recorded draw pipeline have actually retired.
 * A failed retirement keeps this charge in the existing quarantine owner. close
 * is idempotent and releases only this reservation; it is never a renderer ACK. */
public interface SessionResourceCharge {
    public fun close(): Unit
}
