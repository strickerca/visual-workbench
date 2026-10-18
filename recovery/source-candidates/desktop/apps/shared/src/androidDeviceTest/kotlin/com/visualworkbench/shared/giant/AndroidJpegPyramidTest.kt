package com.visualworkbench.shared.giant

import android.graphics.*
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

public class AndroidJpegPyramidTest {
    private fun root(): File = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.canonicalFile, "giant-test-${UUID.randomUUID()}").also { check(it.mkdir()) }
    private fun jpeg(root: File): File {
        val bitmap = Bitmap.createBitmap(513, 257, Bitmap.Config.ARGB_8888)
        try {
            val pixels = IntArray(513 * 257) { i -> Color.rgb(i % 251, (i / 513) % 241, (i / 7) % 239) }
            bitmap.setPixels(pixels, 0, 513, 0, 0, 513, 257)
            return File(root, "original.jpg").also { file -> FileOutputStream(file).use { check(bitmap.compress(Bitmap.CompressFormat.JPEG, 95, it)); it.fd.sync() } }
        } finally { bitmap.recycle() }
    }
    private fun input(file: File, orientation: Int = 1): JpegTileInput = JpegTileInput(file.path,
        MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString("") { "%02x".format(it.toInt() and 255) },513,257,orientation,file.length())
    private suspend fun refuse(block: suspend () -> Unit): TileBuildFailure {
        try { block() } catch (failure: TileBuildFailure) { return failure }
        throw AssertionError("expected refusal")
    }
    @Test public fun rawBandInverseCoversExactPhysicalSourceForAllEightOrientations(): Unit {
        for (orientation in 1..8) {
            val width = if (orientation >= 5) 257 else 513
            val height = if (orientation >= 5) 513 else 257
            var area = 0L
            for (y in 0 until height step 256) {
                val r = AndroidJpegPyramid.rawRect(0,y,width,minOf(256,height-y),513,257,orientation)
                assertTrue(r.left >= 0 && r.right <= 513 && r.top >= 0 && r.bottom <= 257)
                area += r.width().toLong() * r.height()
            }
            assertEquals(513L * 257, area)
        }
    }
    @Test public fun actualLosslessBaseMatchesDecodedJpegAcrossOddRotatedBandEdges(): Unit = runBlocking {
        val root = root(); val work = File(root,"work").also { check(it.mkdir()) }; val file = jpeg(root)
        val before = file.readBytes()
        try {
            for (orientation in listOf(1,2,5,6,7,8)) {
                val build = AndroidJpegPyramid.build(input(file,orientation),work) { }
                try {
                    val raw = BitmapFactory.decodeFile(file.path)
                    val expected = AndroidJpegPyramid.orient(raw,orientation)
                    try {
                        assertEquals(expected.width,build.width); assertEquals(expected.height,build.height)
                        for (y in 0 until build.height step 256) for (x in 0 until build.width step 256) {
                            val tile = BitmapFactory.decodeFile(File(build.directory,"L0/${x/256}_${y/256}.webp").path)
                            try {
                                val actual = IntArray(tile.width*tile.height); val wanted = IntArray(actual.size)
                                tile.getPixels(actual,0,tile.width,0,0,tile.width,tile.height)
                                expected.getPixels(wanted,0,tile.width,x,y,tile.width,tile.height)
                                assertArrayEquals(wanted,actual)
                            } finally { tile.recycle() }
                        }
                        assertFalse(build.directory.listFiles()!!.any { it.name.startsWith("R") })
                        val last = build.directory.listFiles()!!.filter { it.isDirectory }.maxBy { it.name.drop(1).toInt() }
                        val smallest = BitmapFactory.decodeFile(File(last,"0_0.jpg").path)
                        try { assertEquals(1,smallest.width); assertEquals(1,smallest.height) } finally { smallest.recycle() }
                    } finally { if (expected !== raw) expected.recycle(); raw.recycle() }
                } finally { build.close() }
                assertTrue(work.listFiles()!!.isEmpty())
            }
            assertArrayEquals(before,file.readBytes())
        } finally { check(file.delete()); check(work.delete()); check(root.delete()) }
    }
    @Test public fun cancellationRetainsTheActualProducerAndItsSlotUntilSettlement(): Unit = runBlocking {
        val root=root();val work=File(root,"work").also{check(it.mkdir())};val file=jpeg(root)
        val entered=CountDownLatch(1);val release=CountDownLatch(1)
        val operation=async(Dispatchers.Default){AndroidJpegPyramid.build(input(file),work){p->if(p.phase==TileBuildPhase.Preview){entered.countDown();check(release.await(5,TimeUnit.SECONDS))}}}
        try {
            assertTrue(withContext(Dispatchers.IO){entered.await(5,TimeUnit.SECONDS)})
            operation.cancel();yield();assertFalse(operation.isCompleted)
            assertEquals("busy",refuse{AndroidJpegPyramid.build(input(file),work){}}.reason)
        } finally {release.countDown();withContext(NonCancellable){try{operation.await()}catch(_:CancellationException){}}}
        try {assertTrue(work.listFiles()!!.isEmpty());val retry=AndroidJpegPyramid.build(input(file),work){};retry.close()}
        finally {check(file.delete());check(work.delete());check(root.delete())}
    }
    @Test public fun callbackFailureCleansOnlyItsOwnedStagingAndPreservesOriginal(): Unit = runBlocking {
        val root=root();val work=File(root,"work").also{check(it.mkdir())};val file=jpeg(root);val unrelated=File(work,"keep.txt").also{it.writeText("keep")}
        try {
            try {AndroidJpegPyramid.build(input(file),work){throw IllegalStateException("fixture callback")};fail("expected callback failure")}catch(_:IllegalStateException){}
            assertEquals(listOf(unrelated),work.listFiles()!!.toList());assertTrue(file.length()>0)
        }finally{check(unrelated.delete());check(file.delete());check(work.delete());check(root.delete())}
    }
    @Test public fun sourceHashAndBudgetRefuseBeforeAnyStagingPublication(): Unit = runBlocking {
        val root=root();val work=File(root,"work").also{check(it.mkdir())};val file=jpeg(root)
        try {
            assertEquals("source hash",refuse{AndroidJpegPyramid.build(input(file).copy(sha256="0".repeat(64)),work){}}.reason)
            assertEquals("disk budget",refuse{AndroidJpegPyramid.build(input(file).copy(maxDiskBytes=1024),work){}}.reason)
            assertTrue(work.listFiles()!!.isEmpty())
        }finally{check(file.delete());check(work.delete());check(root.delete())}
    }
    @Test public fun taggedJpegNeverSilentlyLosesItsProfile(): Unit = runBlocking {
        val root=root();val work=File(root,"work").also{check(it.mkdir())};val file=jpeg(root)
        val bytes=file.readBytes();val tag=byteArrayOf(0xff.toByte(),0xe2.toByte(),0,14)+"ICC_PROFILE\u0000".toByteArray(Charsets.US_ASCII)
        file.writeBytes(bytes.copyOfRange(0,2)+tag+bytes.copyOfRange(2,bytes.size))
        try {assertEquals("ICC tile conversion unavailable",refuse{AndroidJpegPyramid.build(input(file),work){}}.reason);assertTrue(work.listFiles()!!.isEmpty())}
        finally{check(file.delete());check(work.delete());check(root.delete())}
    }
    @Test public fun cleanupRefusesForeignEntriesAndRetainsOwnerForExplicitRetry(): Unit = runBlocking {
        val root=root();val work=File(root,"work").also{check(it.mkdir())};val file=jpeg(root)
        try {
            val result=AndroidJpegPyramid.build(input(file),work){};val foreign=File(result.directory,"unrelated.txt").also{it.writeText("keep")}
            assertEquals("cleanup foreign entry",refuse{result.close()}.reason);assertEquals("keep",foreign.readText());assertTrue(File(result.directory,".owner").exists())
            check(foreign.delete());result.close();result.close();assertTrue(work.listFiles()!!.isEmpty())
        }finally{check(file.delete());check(work.delete());check(root.delete())}
    }
    @Test public fun oddLongEdgeUsesCeilingSampleAndPublishesAnActuallyBoundedPreview(): Unit = runBlocking {
        val root=root();val work=File(root,"work").also{check(it.mkdir())};val file=File(root,"odd.jpg")
        val bitmap=Bitmap.createBitmap(4097,3,Bitmap.Config.ARGB_8888)
        try{bitmap.eraseColor(Color.BLUE);FileOutputStream(file).use{check(bitmap.compress(Bitmap.CompressFormat.JPEG,95,it));it.fd.sync()}}finally{bitmap.recycle()}
        try{
            assertEquals(4,AndroidJpegPyramid.previewSample(4097,3))
            val input=JpegTileInput(file.path,MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString(""){"%02x".format(it.toInt() and 255)},4097,3,1,file.length())
            val result=AndroidJpegPyramid.build(input,work){}
            try{
                assertTrue(maxOf(result.previewWidth,result.previewHeight)<=2048)
                val preview=BitmapFactory.decodeFile(File(result.directory,"preview.jpg").path)
                try{assertEquals(result.previewWidth,preview.width);assertEquals(result.previewHeight,preview.height);assertTrue(maxOf(preview.width,preview.height)<=2048)}finally{preview.recycle()}
            }finally{result.close()}
        }finally{check(file.delete());check(work.delete());check(root.delete())}
    }
    @Test public fun previewAdmissionRejectsUnexpectedDecoderExtentBeforePublication(): Unit {
        assertEquals(1,AndroidJpegPyramid.previewSample(2048,1));assertEquals(2,AndroidJpegPyramid.previewSample(2049,1))
        assertEquals(512,AndroidJpegPyramid.previewSample(1_048_576,1))
        val bitmap=Bitmap.createBitmap(2049,1,Bitmap.Config.ARGB_8888)
        try{try{AndroidJpegPyramid.admitPreview(bitmap);fail("expected extent refusal")}catch(error:TileBuildFailure){assertEquals("preview extent",error.reason)}}finally{bitmap.recycle()}
    }

}
