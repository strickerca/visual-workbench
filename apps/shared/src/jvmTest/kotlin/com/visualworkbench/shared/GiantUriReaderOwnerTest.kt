package com.visualworkbench.shared

import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Test

class GiantUriReaderOwnerTest {
    @Test fun holderCreationDoesNotOpenAndOpenIsSingleAttempt() {
        val opens=AtomicInteger();val reader=GiantUriReaderOwner { opens.incrementAndGet();ByteArrayInputStream(byteArrayOf(21,42,84)) }
        assertEquals(0,opens.get());reader.open();assertEquals(1,opens.get())
        assertThrows(IllegalStateException::class.java){reader.open()}
        val bytes=ByteArray(64*1024);assertEquals(3,reader.read(bytes));assertArrayEquals(byteArrayOf(21,42,84),bytes.copyOf(3))
        assertEquals(-1,reader.read(bytes));reader.close();reader.close();assertEquals(1,opens.get())
    }

    @Test fun failedCloseRetainsTheExactStreamUntilActualRetry() {
        val closes=AtomicInteger();var fail=true
        val exact=object:ByteArrayInputStream(byteArrayOf(9)) {
            override fun close(){closes.incrementAndGet();if(fail)throw IOException("synthetic close failure");super.close()}
        }
        val reader=GiantUriReaderOwner { exact };reader.open()
        assertThrows(IOException::class.java){reader.close()};assertEquals(1,closes.get())
        fail=false;reader.close();assertEquals(2,closes.get());reader.close();assertEquals(2,closes.get())
        assertThrows(IllegalStateException::class.java){reader.read(ByteArray(64*1024))}
    }

    @Test fun throwingSourceOpenIsNotRetriedAndHasNoReturnedStreamToClose() {
        val opens=AtomicInteger();val reader=GiantUriReaderOwner { opens.incrementAndGet();throw IOException("synthetic open failure") }
        assertThrows(IOException::class.java){reader.open()};reader.close()
        assertThrows(IllegalStateException::class.java){reader.open()};assertEquals(1,opens.get())
    }

    @Test fun readFailureStillLeavesTheSameOwnerForClose() {
        val closes=AtomicInteger();val reader=GiantUriReaderOwner { object:InputStream() {
            override fun read():Int=throw IOException("synthetic read failure")
            override fun close(){closes.incrementAndGet()}
        } }
        reader.open();assertThrows(IOException::class.java){reader.read(ByteArray(64*1024))}
        assertEquals(0,closes.get());reader.close();assertEquals(1,closes.get())
    }

    @Test fun nativeReaderBufferIsFixedAndBounded() {
        val reader=GiantUriReaderOwner { ByteArrayInputStream(byteArrayOf(1)) };reader.open()
        for(size in listOf(0,1,65535,65537))assertThrows(IllegalArgumentException::class.java){reader.read(ByteArray(size))}
        reader.close()
    }

    @Test fun closeWaitsForTheActualInFlightRead() {
        val entered=CountDownLatch(1);val finish=CountDownLatch(1);val closeStarted=CountDownLatch(1)
        val streamClosed=AtomicBoolean(false);val reader=GiantUriReaderOwner { object:InputStream() {
            override fun read():Int {entered.countDown();check(finish.await(5,TimeUnit.SECONDS));return -1}
            override fun close(){streamClosed.set(true)}
        } }
        val threads=Executors.newFixedThreadPool(2);reader.open()
        try {
            val reading=threads.submit<Int>{reader.read(ByteArray(64*1024))};assertTrue(entered.await(5,TimeUnit.SECONDS))
            val closing=threads.submit{closeStarted.countDown();reader.close()};assertTrue(closeStarted.await(5,TimeUnit.SECONDS))
            assertFalse(streamClosed.get());assertFalse(closing.isDone)
            finish.countDown();assertEquals(-1,reading.get(5,TimeUnit.SECONDS));closing.get(5,TimeUnit.SECONDS)
            assertTrue(streamClosed.get())
        } finally {finish.countDown();threads.shutdownNow();assertTrue(threads.awaitTermination(5,TimeUnit.SECONDS));reader.close()}
    }

    @Test fun uriOptionsNeverPrintPrivateSourceOrWorkDirectory() {
        val value=GiantUriOptions("content://synthetic/private-original","/synthetic/private-work",1024uL,8192uL,true)
        assertEquals("GiantUriOptions",value.toString())
    }
}
