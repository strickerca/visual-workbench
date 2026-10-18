package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class DocumentTileOwnershipTest {
    private fun binding(generation: Long = 1) = DocumentFrameBinding("a".repeat(64), "doc", 0u,
        ProjectInfo("project", "title", "device", 1uL, false, false, 1uL, "state", listOf("doc")), 1, generation)
    private fun tile(scale: Double = 1.0, width: UInt = 1u, bytes: ByteArray = byteArrayOf(10, 20, 30, 128.toByte())) = DocumentDisplayTile(Point(0.0, 0.0), scale, width, 1u, bytes)
    private fun frame(tile: DocumentDisplayTile = tile(), generation: Long = 1) = DocumentDisplayFrame(binding(generation), listOf(tile))
    private fun failure(kind: DocumentFailureKind, block: () -> Unit) {
        try { block(); fail("expected fixed failure") } catch (error: DocumentFailure) { assertEquals(kind, error.kind) }
    }
    @Test fun straightRgbaChannelsHaveIndependentKnownArgbAnswer() {
        assertEquals(0x800a141e.toInt(), documentArgb(byteArrayOf(10, 20, 30, 128.toByte()), 0))
        assertEquals(0xff010203.toInt(), documentArgb(byteArrayOf(1, 2, 3, 255.toByte()), 0))
    }
    @Test fun cacheRetirementDoesNotRecycleAnImageStillOwnedByADrawFrame() {
        var disposed = 0
        val images = DocumentTileImages<Int>(10000) { _, _ -> 7 to { disposed++ } }
        val prepared = images.prepare(frame()) { true }
        images.close(); assertEquals(0, disposed); assertEquals(7, prepared.single().value)
        assertTrue(images.retainedBytes() > 0)
        prepared.single().close(); prepared.single().close()
        assertEquals(1, disposed); assertEquals(0, images.retainedBytes())
    }
    @Test fun oldGenerationCannotBeReusedAndItsLeaseStillCountsAgainstBudget() {
        var disposed = 0; var converted = 0
        val images = DocumentTileImages<Int>(4112) { _, _ -> (++converted) to { disposed++ } }
        val original = tile(); val old = images.prepare(frame(original, 1)) { true }
        failure(DocumentFailureKind.Limit) { images.prepare(frame(original, 2)) { true } }
        assertEquals(0, disposed); assertEquals(1, converted)
        old.single().close()
        val next = images.prepare(frame(original, 2)) { true }
        assertEquals(2, converted); assertEquals(2L, next.single().binding.viewportGeneration)
        next.single().close(); images.close(); assertEquals(2, disposed)
    }
    @Test fun staleAfterConversionDisposesOnlyTheUnpublishedImage() {
        var current = true; var disposed = 0
        val images = DocumentTileImages<Int>(10000) { _, _ -> current = false; 7 to { disposed++ } }
        failure(DocumentFailureKind.Stale) { images.prepare(frame()) { current } }
        assertEquals(1, disposed); assertEquals(0, images.retainedBytes())
    }
    @Test fun cancelledBitmapPreparationCannotLoseAcquiredImagesAcrossDispatcherReturn() = runBlocking {
        withTimeout(3000) {
            val created = CompletableDeferred<Unit>(); val release = CountDownLatch(1); var disposed = 0
            val images = DocumentTileImages<Int>(10000) { _, _ ->
                created.complete(Unit)
                check(release.await(2, TimeUnit.SECONDS))
                1 to { disposed++ }
            }
            val operation = async { images.prepareOwned(frame()) { true } }
            created.await(); operation.cancel(); release.countDown(); operation.join()
            images.close(); assertEquals(1, disposed); assertEquals(0, images.retainedBytes())
        }
    }
    @Test fun malformedFramesRefuseBeforeForeignAllocation() {
        var calls = 0
        val images = DocumentTileImages<Int>(10000) { _, _ -> calls++; 7 to {} }
        for (bad in listOf(tile(width = 0u), tile(width = UInt.MAX_VALUE), tile(bytes = byteArrayOf(1)), tile(scale = Double.MIN_VALUE), tile(scale = Double.MAX_VALUE)))
            failure(DocumentFailureKind.Verification) { images.prepare(frame(bad)) { true } }
        val many = DocumentDisplayFrame(binding(), List(33) { tile() })
        failure(DocumentFailureKind.Limit) { images.prepare(many) { true } }
        assertEquals(0, calls)
    }
    @Test fun currentAuthorityIsCheckedBeforeAndAfterPreparation() {
        var calls = 0
        val images = DocumentTileImages<Int>(10000) { _, _ -> calls++; 1 to {} }
        failure(DocumentFailureKind.Stale) { images.prepare(frame()) { false } }
        assertEquals(0, calls)
    }
    @Test fun failedDisposalRetainsTheExactCacheOwnerForCloseRetry() {
        var fail = true; var attempts = 0
        val images = DocumentTileImages<Int>(10000) { _, _ -> 1 to { attempts++; if (fail) error("synthetic disposal") } }
        val ready = images.prepare(frame()) { true }; ready.single().close()
        failure(DocumentFailureKind.Isolation) { images.close() }
        assertTrue(images.retainedBytes() > 0)
        fail = false; images.close(); assertEquals(2, attempts); assertEquals(0, images.retainedBytes())
    }
}
