package com.visualworkbench.shared

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantExportRetirementTest {
    @Test fun exportStartsOnlyAfterActualRendererAcknowledgesAndTileCreditReturns() = runBlocking {
        val source = Source(); val backend = Backend(); val owner = GiantViewportSession(this, source, backend)
        val entered = CompletableDeferred<Unit>(); val released = CompletableDeferred<Unit>()
        var exporting: Deferred<Unit>? = null
        try {
            owner.request(view(0.0)); until { owner.state.value.ready }
            backend.retiring = { entered.complete(Unit); released.await() }
            exporting = async { owner.withRetiredImage {
                assertTrue(backend.freed); assertEquals(0L, owner.state.value.retainedBytes); source.exports++
            } }
            withTimeout(3000) { entered.await() }
            assertEquals(0, source.exports); assertFalse(backend.freed)
            released.complete(Unit); withTimeout(3000) { checkNotNull(exporting).await() }
            assertEquals(1, source.exports)
        } finally { released.complete(Unit); withContext(NonCancellable) { exporting?.join(); owner.close() } }
    }
    @Test fun viewportChangesDuringExportRestoreOnlyLatestGeneration() = runBlocking {
        val source = Source(); val owner = GiantViewportSession(this, source, Backend())
        val entered = CompletableDeferred<Unit>(); val released = CompletableDeferred<Unit>()
        var exporting: Job? = null
        try {
            owner.request(view(0.0)); until { owner.state.value.ready }
            exporting = launch { owner.withRetiredImage { entered.complete(Unit); released.await() } }
            withTimeout(3000) { entered.await() }
            owner.request(view(1.0)); owner.request(view(2.0)); owner.request(view(3.0))
            released.complete(Unit); checkNotNull(exporting).join(); until { owner.state.value.ready }
            assertEquals(listOf(0.0, 3.0), source.plans)
        } finally { released.complete(Unit); withContext(NonCancellable) { exporting?.join(); owner.close() } }
    }
    @Test fun closeJoinsCancelledExportAndDoesNotRestartItsRenderer() = runBlocking {
        val source = Source(); val backend = Backend(); val owner = GiantViewportSession(this, source, backend)
        val entered = CompletableDeferred<Unit>(); val released = CompletableDeferred<Unit>()
        val exporting = launch { owner.withRetiredImage { withContext(NonCancellable) { entered.complete(Unit); released.await() } } }
        var closing: Job? = null
        try {
            withTimeout(3000) { entered.await() }; exporting.cancel()
            closing = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
            assertFalse(checkNotNull(closing).isCompleted); assertFalse(source.closed)
            owner.request(view(1.0)); assertTrue(source.plans.isEmpty())
        } finally { released.complete(Unit); withContext(NonCancellable) { exporting.join(); closing?.join(); owner.close() } }
        assertTrue(source.closed); assertNull(backend.frame)
    }
    private class Source : GiantImage {
        val plans = mutableListOf<Double>(); var exports = 0; var closed = false
        override suspend fun viewport(view: GiantViewport): GiantViewportPlan {
            plans += view.left
            return GiantViewportPlan("a".repeat(64), "b".repeat(64), 0u, listOf(GiantTileAddress(0u, 0u, 0u)), emptyList())
        }
        override suspend fun tile(address: GiantTileAddress) = GiantTilePixels("a".repeat(64), "b".repeat(64), address,
            0u, 0u, 1u, 1u, 1u, 1u, byteArrayOf(0, 0, 0, (-1).toByte()))
        override suspend fun exportPng(options: GiantPngOptions): GiantPngReceipt = error("test supplies the real ownership block")
        override suspend fun close() { closed = true }
    }
    private class Backend : GiantRenderBackend<Int> {
        var retiring: suspend () -> Unit = { }; var freed = false; var frame: GiantVisibleFrame<Int>? = null
        override suspend fun create(pixels: GiantTilePixels) = 1
        override fun present(frame: GiantVisibleFrame<Int>?) { this.frame = frame }
        override suspend fun retire() { retiring() }
        override fun release(image: Int) { freed = true }
    }
    private suspend fun until(check: () -> Boolean) = withTimeout(3000) { while (!check()) yield() }
    private fun view(x: Double) = GiantViewport(x, 0.0, x + 1, 1.0, 1.0, 0.0, 0.0, 1.0)
}
