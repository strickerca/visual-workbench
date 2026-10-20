package com.visualworkbench.android.stream

import android.graphics.Color
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class AndroidStreamPixelsInstrumentedTest {
    @Test fun admissionPrecedesRowsAndCopiesExactOpaquePixelsInBoundedBatches() = runBlocking {
        val pool = Pool(); val source = Rows(pool); val factory = AndroidStreamPixelFactory(pool)
        val image = factory.copy(source)
        assertEquals(listOf(16u, 16u, 5u), source.rows)
        assertEquals(Color.rgb(2, 3, 5), image.value.getPixel(2, 3))
        assertEquals(1, pool.held); assertEquals(0, source.closes)
        image.close(); factory.close(); assertEquals(0, pool.held)
    }
    @Test fun malformedRowsReleaseOnlyThePartialBitmapAndItsCharge() = runBlocking {
        val pool = Pool(); val source = Rows(pool); source.malformed = true
        val factory = AndroidStreamPixelFactory(pool)
        assertTrue(runCatching { factory.copy(source) }.isFailure)
        assertEquals(0, pool.held); assertEquals(0, source.closes); factory.close()
    }
    @Test fun cancellationWhileReadingRowsJoinsPartialBitmapCleanup() = runBlocking {
        val pool = Pool(); val source = Rows(pool)
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        source.beforeRows = { entered.complete(Unit); release.await() }
        val factory = AndroidStreamPixelFactory(pool)
        val copying = launch { factory.copy(source) }
        entered.await(); copying.cancelAndJoin()
        assertEquals(0, pool.held); factory.close()
    }
    @Test fun refusedForeignReservationNeverReadsPixels() = runBlocking {
        val pool = Pool(); pool.refuse = true
        val source = Rows(pool); val factory = AndroidStreamPixelFactory(pool)
        assertTrue(runCatching { factory.copy(source) }.isFailure)
        assertTrue(source.rows.isEmpty()); assertEquals(0, pool.held); factory.close()
    }

    private class Pool : SessionResources {
        var held = 0; var refuse = false
        override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge {
            check(!refuse); check(bytes >= 4096uL * 37uL * 4uL + 512uL * 1024uL); held++
            return object : SessionResourceCharge {
                var closed = false
                override fun close() { if (!closed) { closed = true; held-- } }
            }
        }
    }
    private class Rows(private val pool: Pool) : CaptureStreamPixels {
        override val frame = CaptureStreamFrame("capture", 1uL, 1uL, 4096u, 37u, 0, 0, 1.0, 1u, 1, 1000, "windows")
        val rows = mutableListOf<UInt>(); var closes = 0; var malformed = false
        var beforeRows: suspend () -> Unit = {}
        override suspend fun rgbaRows(firstRow: UInt, rowCount: UInt): ByteArray {
            check(pool.held == 1); beforeRows(); rows += rowCount
            if (malformed) return byteArrayOf(1)
            val bytes = ByteArray((frame.width * rowCount * 4u).toInt())
            for (row in 0 until rowCount.toInt()) for (x in 0 until frame.width.toInt()) {
                val index = (row * frame.width.toInt() + x) * 4
                val y = firstRow.toInt() + row
                bytes[index] = x.toByte(); bytes[index + 1] = y.toByte(); bytes[index + 2] = (x + y).toByte(); bytes[index + 3] = 255.toByte()
            }
            return bytes
        }
        override suspend fun close() { closes++ }
    }
}
