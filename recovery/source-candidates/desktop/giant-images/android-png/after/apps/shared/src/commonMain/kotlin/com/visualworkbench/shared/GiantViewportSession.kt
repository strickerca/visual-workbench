package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** A renderer epoch owns EVERY image ever handed to its recorded draw pipeline.
 * retire() must detach and destroy that pipeline before returning. Clearing a
 * UI list or closing an image wrapper is not a retirement acknowledgment. */
public interface GiantRenderBackend<T : Any> {
    public suspend fun create(pixels: GiantTilePixels): T
    public fun present(frame: GiantVisibleFrame<T>?)
    public suspend fun retire()
    public fun release(image: T)
}
public data class GiantRenderedTile<T : Any>(public val pixels: GiantTileDescriptor, public val image: T)
public data class GiantTileDescriptor(public val address: GiantTileAddress, public val x: UInt, public val y: UInt,
    public val documentWidth: UInt, public val documentHeight: UInt, public val width: UInt, public val height: UInt)
public data class GiantVisibleFrame<T : Any>(public val generation: ULong, public val view: GiantViewport,
    public val assetId: String, public val manifestHash: String, public val tiles: List<GiantRenderedTile<T>>)
public data class GiantViewStatus(public val generation: ULong = 0uL, public val ready: Boolean = false,
    public val busy: Boolean = false, public val retainedBytes: Long = 0L, public val message: String? = null)

/** UI-dispatcher confined. One latest viewport, one actual native read and one
 * joined lifetime. The 192 MiB default accounts a CPU and GPU copy per tile plus
 * bookkeeping; the platform separately reserves its surfaces/native cache.
 * Prefetch uses the same epoch budget. Source identity is locked by the first
 * authenticated native plan and can never change within this owner. */
public class GiantViewportSession<T : Any>(
    parent: CoroutineScope, private val image: GiantImage, private val backend: GiantRenderBackend<T>,
    private val maxRetainedBytes: Long = 192L * 1024L * 1024L,
) {
    private data class Request(val generation: ULong, val view: GiantViewport)
    private val supervisor: CompletableJob = SupervisorJob(parent.coroutineContext[Job])
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + supervisor)
    private val operationGate: Mutex = Mutex()
    private val requests: Channel<Request> = Channel(Channel.CONFLATED)
    private val mutable: MutableStateFlow<GiantViewStatus> = MutableStateFlow(GiantViewStatus())
    public val state: StateFlow<GiantViewStatus> = mutable
    private val retained: MutableMap<GiantTileAddress, GiantRenderedTile<T>> = linkedMapOf()
    private var bytes: Long = 0L
    private var generation: ULong = 0uL
    private var lastView: GiantViewport? = null
    private var identity: Pair<String, String>? = null
    private var sealed: Boolean = false
    private var closeFailure: Throwable? = null
    private val runner: Job
    init {
        require(maxRetainedBytes in TILE_CHARGE..192L * 1024L * 1024L)
        // UNDISPATCHED enters try/finally before this owner is returned; even a
        // pre-cancelled parent therefore cannot strand image/backend ownership.
        runner = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            try {
                for (request in requests) {
                    try { operationGate.withLock { load(request) } }
                    catch (cancel: CancellationException) { throw cancel }
                    catch (_: Exception) {
                        operationGate.withLock { retire() }
                        if (request.generation == generation) mutable.value = mutable.value.copy(
                            busy = false, ready = false, message = "Derived tiles are unavailable or exceed the admitted viewport budget. Rebuild the cache from the preserved original.")
                    }
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { closeFailure = error }
            finally {
                withContext(NonCancellable) {
                    sealed = true
                    operationGate.withLock {
                        try { retire() } catch (error: Throwable) { closeFailure = error }
                        try { image.close() } catch (error: Throwable) { if (closeFailure == null) closeFailure = error }
                    }
                    mutable.value = mutable.value.copy(busy = false, ready = false)
                }
            }
        }
        runner.invokeOnCompletion { supervisor.complete() }
    }
    public fun request(view: GiantViewport): Unit {
        if (sealed || !runner.isActive || view == lastView) return
        require(listOf(view.left, view.top, view.right, view.bottom, view.a, view.b, view.c, view.d).all(Double::isFinite))
        require(view.right > view.left && view.bottom > view.top && generation < ULong.MAX_VALUE)
        lastView = view; generation++
        mutable.value = mutable.value.copy(generation = generation, ready = false, busy = true, message = null)
        requests.trySend(Request(generation, view))
    }
    public fun retry(): Unit { val prior = lastView ?: return; lastView = null; request(prior) }
    private fun current(request: Request): Boolean = !sealed && request.generation == generation
    private suspend fun retire(): Unit {
        backend.present(null)
        // If retirement fails, keep every tile and its charge. Releasing it
        // would falsely credit references still owned by the rendering context.
        backend.retire()
        val iterator = retained.entries.iterator()
        try {
            while (iterator.hasNext()) {
                val entry = iterator.next()
                backend.release(entry.value.image); iterator.remove(); bytes -= TILE_CHARGE
            }
        } finally { mutable.value = mutable.value.copy(retainedBytes = bytes, ready = false) }
    }
    private suspend fun load(request: Request): Unit {
        if (!current(request)) return
        val plan = image.viewport(request.view)
        if (!current(request)) return
        val pair = plan.assetId to plan.manifestHash
        require(listOf(plan.assetId, plan.manifestHash).all { value -> value.length == 64 && value.all { it in '0'..'9' || it in 'a'..'f' } })
        if (identity == null) identity = pair else require(identity == pair)
        require(plan.visible.size <= 4096 && plan.prefetch.size <= 4096)
        val visible = plan.visible.toSet(); val prefetch = plan.prefetch.toSet()
        require(visible.size == plan.visible.size && prefetch.size == plan.prefetch.size && visible.intersect(prefetch).isEmpty())
        require(visible.size + prefetch.size <= 4096 && (visible + prefetch).all { it.level == plan.level })
        require(visible.size.toLong() * TILE_CHARGE <= maxRetainedBytes)
        val missing = visible.count { it !in retained }
        if (bytes + missing.toLong() * TILE_CHARGE > maxRetainedBytes) retire()
        if (!current(request)) return
        suspend fun read(key: GiantTileAddress): Boolean {
            if (key in retained) return true
            if (bytes + TILE_CHARGE > maxRetainedBytes) return false
            // Reserve BEFORE either the foreign byte array or bitmap exists.
            bytes += TILE_CHARGE; mutable.value = mutable.value.copy(retainedBytes = bytes)
            var created: T? = null; var transferred = false
            try {
                val pixels = image.tile(key)
                require(pixels.assetId == pair.first && pixels.manifestHash == pair.second && pixels.address == key)
                require(pixels.width in 1u..256u && pixels.height in 1u..256u)
                require(pixels.rgba.size.toLong() == pixels.width.toLong() * pixels.height.toLong() * 4L)
                require(pixels.documentWidth > 0u && pixels.documentHeight > 0u)
                require(pixels.documentX.toULong() + pixels.documentWidth.toULong() <= 1_048_576uL && pixels.documentY.toULong() + pixels.documentHeight.toULong() <= 1_048_576uL)
                require((3 until pixels.rgba.size step 4).all { pixels.rgba[it] == (-1).toByte() })
                if (!current(request)) return false
                val value = backend.create(pixels); created = value
                if (!current(request)) return false
                retained[key] = GiantRenderedTile(GiantTileDescriptor(key, pixels.documentX, pixels.documentY,
                    pixels.documentWidth, pixels.documentHeight, pixels.width, pixels.height), value)
                transferred = true; return true
            } finally {
                if (!transferred) { created?.let(backend::release); bytes -= TILE_CHARGE; mutable.value = mutable.value.copy(retainedBytes = bytes) }
            }
        }
        for (key in plan.visible) { currentCoroutineContext().ensureActive(); if (!current(request) || !read(key)) return }
        if (!current(request)) return
        backend.present(GiantVisibleFrame(request.generation, request.view, pair.first, pair.second,
            plan.visible.map { checkNotNull(retained[it]) }))
        mutable.value = mutable.value.copy(ready = true, busy = false, message = null)
        for (key in plan.prefetch) { currentCoroutineContext().ensureActive(); if (!current(request) || !read(key)) break }
    }
    /** Serialize export with viewport reads. The borrowed handle must not escape
     * this block. close seals admission and joins this guard before native close. */
    public suspend fun <R> withExclusiveImage(block: suspend (GiantImage) -> R): R = operationGate.withLock {
        currentCoroutineContext().ensureActive()
        check(!sealed && runner.isActive)
        block(image)
    }
    /** Retire the complete foreign rendering epoch before a bounded native
     * export. While the guard is held, viewport requests only replace the latest
     * request. The same actual close guard joins this operation. */
    public suspend fun <R> withRetiredImage(block: suspend (GiantImage) -> R): R = operationGate.withLock {
        currentCoroutineContext().ensureActive()
        check(!sealed && runner.isActive)
        try {
            retire()
            currentCoroutineContext().ensureActive()
            block(image)
        } finally {
            if (!sealed && runner.isActive) retry()
        }
    }
    /** Every closer joins the same worker and native/renderer teardown, including
     * cancellation during a native read. Never call project.close before this. */
    public suspend fun close(): Unit {
        sealed = true; requests.close(); runner.cancel()
        withContext(NonCancellable) { runner.join(); closeFailure?.let { throw it } }
    }
    public companion object {
        /** Two 256Ãƒâ€šÃ‚Â² RGBA copies plus 16 KiB per-image/index allowance. Native
         * codec/FFI scratch and platform surfaces are charged separately. */
        public const val TILE_CHARGE: Long = 256L * 256L * 8L + 16L * 1024L
    }
}
