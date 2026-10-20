package com.visualworkbench.android.editor

/** Called under the Surface session lock. A recorded draw is still in flight
 * until its buffer completion callback; recording alone cannot admit more
 * application work onto the renderer's finite buffer pool.
 *
 * AndroidX also draws without an application request. Count those draws, and
 * never let an anonymous completion acknowledge a request not yet drawn. */
internal class CanvasDryFrames {
    internal class Request
    private var requested: Request? = null
    private var requestDrawn = false
    private var draws = 0
    private var closed = false

    fun request(): Request? {
        if (closed || requested != null || draws != 0) return null
        return Request().also { requested = it; requestDrawn = false }
    }

    fun drawStarted(requests: Collection<Request>) {
        if (closed) return
        check(draws < Int.MAX_VALUE)
        draws++
        val pending = requested
        if (pending != null && requests.any { it === pending }) requestDrawn = true
    }

    fun drawCompleted() {
        if (closed) return
        check(draws > 0)
        draws--
        if (draws == 0 && requestDrawn) {
            requested = null
            requestDrawn = false
        }
    }

    fun close() { closed = true }
}
