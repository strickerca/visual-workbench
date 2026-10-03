@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import java.io.File
import java.util.Properties
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.*
import org.junit.Assert.*

internal object CoreSmokeContract {
    private fun fixture(): Properties = Properties().apply {
        val stream = CoreSmokeContract::class.java.classLoader!!.getResourceAsStream("ffi-golden.properties")
            ?: error("Rust FFI golden is missing; run the reviewed fixture generator")
        stream.use { load(it) }
        check(getProperty("schema") == "1")
    }
    private fun String.bytes(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    suspend fun golden(directory: File) {
        val f=fixture();val core=workbenchCore();val path=directory.resolve("golden-project").absolutePath
        val project=core.create(CreateProject(path,f.getProperty("id1"),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),"FFI deterministic fixture",f.getProperty("source_png_hex").bytes(),f.getProperty("time_ms").toLong()))
        try {
            val stroke=project.beginStroke(StrokeOptions(f.getProperty("id4"),f.getProperty("id5"),f.getProperty("id6"),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),1uL,f.getProperty("time_ms").toLong()+1,"pen",4.0,0x203040ffu))
            try {
                val batch=SampleBatch(1uL,doubleArrayOf(10.0,20.0,30.0),doubleArrayOf(12.0,24.0,18.0),uintArrayOf(0u,8u,16u),floatArrayOf(0.5f,0.75f,1f))
                val first=stroke.append(batch);val retry=stroke.append(batch)
                assertArrayEquals(first.contours.x,retry.contours.x);assertArrayEquals(first.contours.y,retry.contours.y)
                assertEquals(3uL,first.sampleCount)
                val committed=stroke.commit();assertEquals(1uL,committed.hostSeq)
                val output=project.export(ExportOptions(f.getProperty("id2"),assumeUntaggedSrgb=true))
                assertEquals(f.getProperty("state_hash"),output.revision.stateHash)
                assertEquals(f.getProperty("export_blake3"),output.blake3)
                assertArrayEquals(f.getProperty("export_png_hex").bytes(),output.bytes)
                assertEquals(1,project.document(f.getProperty("id2")).render.items.size)
            } finally { stroke.dispose() }
        } finally { project.close() }
        try { project.info(); fail("closed project was usable") } catch (error: CoreFailure) { assertEquals(CoreFailureKind.Closed,error.kind) }
        val reopened=core.open(path)
        try { assertEquals(f.getProperty("state_hash"),reopened.info().stateHash) } finally { reopened.close() }
    }
    suspend fun hundredThousand(directory: File) {
        val f=fixture();val core=workbenchCore();val project=core.create(CreateProject(directory.resolve("boundary-project").absolutePath,f.getProperty("id1"),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),"FFI boundary fixture",f.getProperty("source_png_hex").bytes(),f.getProperty("time_ms").toLong()))
        try {
            val stroke=project.beginStroke(StrokeOptions(f.getProperty("id4"),f.getProperty("id5"),f.getProperty("id6"),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),1uL,f.getProperty("time_ms").toLong()+1,"pen",4.0,0x203040ffu))
            try {
                var total=0;var sequence=1uL
                while(total<100_000) {
                    val n=minOf(512,100_000-total)
                    val input=SampleBatch(sequence,DoubleArray(n){10.0},DoubleArray(n){10.0},UIntArray(n){(total+it).toUInt()},FloatArray(n){0.5f})
                    val update=stroke.append(input);total+=n;sequence++;assertEquals(total.toULong(),update.sampleCount)
                }
                assertEquals(1uL,stroke.commit().hostSeq)
                assertEquals(1uL,stroke.commit().hostSeq)
            } finally { stroke.dispose() }
        } finally { project.close() }
    }
    /** Separates valid inline completion from a suspended acquisition. Queued
     * delivery is cancelled deterministically; actual native producers may win
     * before suspension and are then released normally. */
    private class DeliveryGate:CoroutineDispatcher() {
        val queue=LinkedBlockingQueue<Runnable>()
        override fun dispatch(context:CoroutineContext,block:Runnable) { queue.add(block) }
        suspend fun next():Runnable=withContext(Dispatchers.IO) {
            queue.poll(30,TimeUnit.SECONDS)?:error("Native ownership phase timed out before dispatch")
        }
    }
    private suspend fun <T:Any> cancelBeforeDelivery(acquire:suspend ()->T,release:suspend (T)->Unit,beginProducer:()->Unit={}):Boolean=coroutineScope {
        val gate=DeliveryGate()
        val delivered=java.util.concurrent.atomic.AtomicReference<T?>(null)
        val job=launch(gate) { delivered.set(acquire()) }
        try {
            gate.next().run()
            beginProducer()
            val inline=delivered.getAndSet(null)
            val queued=inline==null
            if(inline!=null) {
                withContext(NonCancellable) { release(inline) }
            } else {
                val delivery=gate.next()
                job.cancel()
                delivery.run()
                job.join()
                assertNull("Cancelled native handle reached its caller",delivered.get())
            }
            drainNativeTransfers()
            queued
        } finally {
            job.cancel()
            while(gate.queue.isNotEmpty())gate.queue.poll()?.run()
            withContext(NonCancellable) {
                delivered.getAndSet(null)?.let { release(it) }
                drainNativeTransfers()
            }
        }
    }
    suspend fun cancelledHandles(directory:File) {
        val f=fixture();val core=workbenchCore();val time=f.getProperty("time_ms").toLong()
        var queued=0
        // More iterations than the native eight-project/four-gesture limits
        // make a leaked returned resource fail a subsequent acquisition.
        repeat(12) { iteration ->
            println("FFI ownership phase ${iteration+1}/12: create/open/six gestures")
            val path=directory.resolve("cancelled-project-$iteration").absolutePath
            if(cancelBeforeDelivery({core.create(CreateProject(path,f.getProperty("id1"),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),"Cancellation fixture",f.getProperty("source_png_hex").bytes(),time))},{it.close()}))queued++
            if(cancelBeforeDelivery({core.open(path)},{it.close()}))queued++
            val project=core.open(path)
            try {
                repeat(6) {
                    if(cancelBeforeDelivery({project.beginStroke(StrokeOptions(core.newId(time.toULong()),core.newId(time.toULong()),core.newId(time.toULong()),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),1uL,time+1,"pen",4.0,0x203040ffu))},{try{it.cancel()}finally{it.dispose()}}))queued++
                    val probe=project.beginStroke(StrokeOptions(core.newId(time.toULong()),core.newId(time.toULong()),core.newId(time.toULong()),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),1uL,time+1,"pen",4.0,0x203040ffu))
                    try { probe.cancel() } finally { probe.dispose() }
                }
                assertEquals(0uL,project.info().hostSeq)
            } finally { project.close() }
        }
        println("FFI real-handle stress queued_cancellations=$queued inline_deliveries=${96-queued}")
    }
    suspend fun cancelledStrokeDisposal(directory:File):Unit=coroutineScope {
        val f=fixture();val core=workbenchCore();val time=f.getProperty("time_ms").toLong()
        val project=core.create(CreateProject(directory.resolve("cancelled-dispose-project").absolutePath,
            f.getProperty("id1"),f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),
            "Cancelled disposal fixture",f.getProperty("source_png_hex").bytes(),time))
        try {
            repeat(6) {
                val stroke=project.beginStroke(StrokeOptions(core.newId(time.toULong()),core.newId(time.toULong()),core.newId(time.toULong()),
                    f.getProperty("id2"),f.getProperty("id3"),f.getProperty("device_id"),1uL,time+1,"pen",4.0,0x203040ffu))
                val entered=CompletableDeferred<Unit>();var cleanupReached=false
                val job=launch {
                    try { entered.complete(Unit);awaitCancellation() }
                    finally { stroke.dispose();cleanupReached=true }
                }
                entered.await();job.cancelAndJoin()
                assertTrue("Cancelled cross-dispatch disposal skipped caller cleanup",cleanupReached)
                stroke.dispose() // Idempotence includes a handle whose native worker has exited.
            }
            assertEquals(0uL,project.info().hostSeq)
        } finally { project.close() }
    }
    suspend fun cancelledSessionOwnership() {
        workbenchCore()
        var inlineReleased=0
        assertFalse(cancelBeforeDelivery({Any()},{inlineReleased++}))
        assertEquals(1,inlineReleased)
        repeat(12) {
            val owner=SessionOwner()
            val releases=java.util.concurrent.atomic.AtomicInteger()
            // A synchronous fixture can resume before suspendCancellableCoroutine
            // actually suspends. Hold it until the caller has returned to its gate.
            val producerReady=CompletableDeferred<Unit>()
            assertTrue("Gated owner acquisition must suspend before delivery",cancelBeforeDelivery({
                acquireOwned(owner,{producerReady.await();Any()},{
                    object:OwnedSession {
                        override suspend fun close() { releaseNative { releases.incrementAndGet();owner.remove(this) } }
                    }
                },{releases.incrementAndGet()})
            },{it.close()},{producerReady.complete(Unit)}))
            // Closing the service after the cancelled delivery must not find a
            // stale registered wrapper and release its native handle twice.
            owner.close();assertEquals(1,releases.get())
        }
    }
    suspend fun cancelledProjectClosure(directory:File):Unit=coroutineScope {
        val f=fixture();val core=workbenchCore()
        val path=directory.resolve("cancelled-close-project").absolutePath
        core.create(CreateProject(path,f.getProperty("id1"),f.getProperty("id2"),f.getProperty("id3"),
            f.getProperty("device_id"),"Cancelled project closure",f.getProperty("source_png_hex").bytes(),
            f.getProperty("time_ms").toLong())).close()
        repeat(6) {
            val project=core.open(path)
            val entered=CompletableDeferred<Unit>();var cleanupReached=false
            val job=launch {
                try { entered.complete(Unit);awaitCancellation() }
                finally { project.close();cleanupReached=true }
            }
            entered.await();job.cancelAndJoin()
            assertTrue("Cancelled project close skipped caller cleanup",cleanupReached)
            project.close()
        }
    }
    fun disposeDirectory(directory: File) {
        check(directory.name.startsWith("vw-ffi-smoke-"))
        check(directory.isDirectory && directory.canonicalFile.parentFile == requireNotNull(directory.absoluteFile.parentFile).canonicalFile)
        check(directory.deleteRecursively()) { "Owned FFI fixture directory remained after close" }
    }
}
