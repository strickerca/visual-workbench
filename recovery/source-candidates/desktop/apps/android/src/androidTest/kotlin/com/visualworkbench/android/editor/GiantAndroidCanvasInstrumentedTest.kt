package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantAndroidCanvasInstrumentedTest {
    @Test fun inlineReleaseWaiterObservesRemovedLeaseAndOneReadmission() = runBlocking {
        withContext(Dispatchers.Main.immediate) {
            var available = 0
            val leases = GiantRenderLeases { available++; assertEquals(0, leases.retainedCount()) }
            val lease = checkNotNull(leases.acquire { })
            val closing = async(Dispatchers.Unconfined, start = CoroutineStart.UNDISPATCHED) { leases.retireAll() }
            try {
                assertFalse(closing.isCompleted)
                lease.released() // Unconfined await resumes synchronously inside complete().
                assertTrue(closing.isCompleted); closing.await()
                lease.released(); assertEquals(1, available)
            } finally { lease.released(); withContext(NonCancellable) { closing.await() } }
        }
    }

    @Test fun rendererLeaseSurvivesDetachUntilExactCallback() = runBlocking {
        var requests = 0; var available = 0
        val leases = GiantRenderLeases { available++ }
        val old = checkNotNull(leases.acquire { requests++ })
        val replacement = checkNotNull(leases.acquire { requests++ })
        assertNull(leases.acquire { error("third renderer") })
        val closing = launch(start = CoroutineStart.UNDISPATCHED) { leases.retireAll() }
        try {
            assertEquals(2, requests); assertFalse(closing.isCompleted)
            assertNull(leases.acquire { error("admitted during retirement") })
            old.released(); old.released()
            assertEquals(1, leases.retainedCount()); assertEquals(0, available)
            assertFalse(closing.isCompleted)
        } finally { replacement.released(); withTimeout(3000) { closing.join() } }
        assertEquals(0, leases.retainedCount()); assertEquals(1, available)
    }

    @Test fun cancelledRetirementStillWaitsForRelease() = runBlocking {
        val leases = GiantRenderLeases { }
        val lease = checkNotNull(leases.acquire { })
        val closing = launch(start = CoroutineStart.UNDISPATCHED) { leases.retireAll() }
        try {
            closing.cancel(); yield()
            assertFalse(closing.isCompleted); assertEquals(1, leases.retainedCount())
        } finally { lease.released(); withContext(NonCancellable) { withTimeout(3000) { closing.join() } } }
        assertEquals(0, leases.retainedCount())
    }

    @Test fun failedReleaseRequestDoesNotCreditOrReadmit() = runBlocking {
        val leases = GiantRenderLeases { }
        val lease = checkNotNull(leases.acquire { throw IllegalStateException("controlled failure") })
        try {
            try { leases.retireAll(); fail("expected failure") } catch (_: IllegalStateException) { }
            assertEquals(1, leases.retainedCount())
            assertNull(leases.acquire { })
        } finally { lease.released() }
    }

    @Test fun actualBitmapStaysLiveUntilAllRecordedRenderersRetire() = runBlocking {
        val source = Source()
        val leases = GiantRenderLeases { }
        val first = checkNotNull(leases.acquire { })
        val second = checkNotNull(leases.acquire { })
        val owner = GiantAndroidCanvas(this, source, leases) { }
        owner.request(view())
        var close: Job? = null
        try {
            until { owner.status.ready }
            val tile = checkNotNull(owner.frame).tiles.single().image
            close = launch(start = CoroutineStart.UNDISPATCHED) { owner.close() }
            yield(); assertFalse(tile.isRecycled); assertFalse(source.closed)
            first.released(); yield(); assertFalse(tile.isRecycled); assertFalse(source.closed)
            second.released(); withTimeout(3000) { checkNotNull(close).join() }
            assertTrue(tile.isRecycled); assertTrue(source.closed)
        } finally {
            first.released(); second.released()
            withContext(NonCancellable) { close?.join(); owner.close() }
        }
    }

    @Test fun opaqueTileBytesAreConvertedExactly() = runBlocking {
        val pixels = tile().copy(width = 2u, height = 1u, rgba = byteArrayOf(255.toByte(), 1, 2, 255.toByte(), 3, 4, 5, 255.toByte()))
        val bitmap = giantAndroidBitmap(pixels)
        try { assertEquals(0xffff0102.toInt(), bitmap.getPixel(0, 0)); assertEquals(0xff030405.toInt(), bitmap.getPixel(1, 0)) }
        finally { bitmap.recycle() }
    }

    @Test fun tileRefusesUnexpectedAlphaOrDimensions() = runBlocking {
        for (bad in listOf(tile().copy(width = 257u), tile().copy(rgba = ByteArray(4)))) {
            try { giantAndroidBitmap(bad).recycle(); fail("invalid tile admitted") } catch (_: IllegalArgumentException) { }
        }
        val transparent = tile().copy(width = 1u, height = 1u, rgba = byteArrayOf(1, 2, 3, 0))
        try { giantAndroidBitmap(transparent).recycle(); fail("alpha admitted") } catch (_: IllegalArgumentException) { }
    }

    @Test fun physicalSurfaceAdmissionUsesAreaAndAxisBounds() {
        admitGiantAndroidSurface(2048, 3072)
        for (size in listOf(2049 to 3072, 8193 to 1, 1 to 0, Int.MAX_VALUE to Int.MAX_VALUE)) {
            try { admitGiantAndroidSurface(size.first, size.second); fail("surface admitted") } catch (_: IllegalArgumentException) { }
        }
    }

    @Test fun realTilePaintPreservesExactDocumentPlacement() {
        val source = Bitmap.createBitmap(2, 1, Bitmap.Config.ARGB_8888)
        val target = Bitmap.createBitmap(8, 3, Bitmap.Config.ARGB_8888)
        try {
            source.setPixel(0, 0, 0xffff0000.toInt()); source.setPixel(1, 0, 0xff00ff00.toInt())
            val frame = GiantVisibleFrame(1uL, view(), "a".repeat(64), "b".repeat(64), listOf(
                GiantRenderedTile(GiantTileDescriptor(GiantTileAddress(1u, 0u, 0u), 2u, 1u, 4u, 1u, 2u, 1u), source)))
            drawGiantAndroidTiles(Canvas(target), frame)
            assertEquals(0, target.getPixel(1, 1)); assertEquals(0xffff0000.toInt(), target.getPixel(2, 1))
            assertEquals(0xffff0000.toInt(), target.getPixel(3, 1)); assertEquals(0xff00ff00.toInt(), target.getPixel(4, 1))
            assertEquals(0xff00ff00.toInt(), target.getPixel(5, 1)); assertEquals(0, target.getPixel(6, 1))
        } finally { source.recycle(); target.recycle() }
    }

    private class Source : GiantImage {
        var closed = false
        override suspend fun viewport(view: GiantViewport) = GiantViewportPlan("a".repeat(64), "b".repeat(64), 0u,
            listOf(GiantTileAddress(0u, 0u, 0u)), emptyList())
        override suspend fun tile(address: GiantTileAddress) = Companion.tile()
        override suspend fun exportPng(options: GiantPngOptions): GiantPngReceipt = error("unused")
        override suspend fun close() { closed = true }
    }
    private suspend fun until(test: () -> Boolean) { withTimeout(3000) { while (!test()) yield() } }
    private companion object {
        fun view() = GiantViewport(0.0, 0.0, 256.0, 256.0, 1.0, 0.0, 0.0, 1.0)
        fun tile() = GiantTilePixels("a".repeat(64), "b".repeat(64), GiantTileAddress(0u, 0u, 0u),
            0u, 0u, 256u, 256u, 256u, 256u, ByteArray(256 * 256 * 4) { if (it % 4 == 3) (-1).toByte() else 0 })
    }
}
