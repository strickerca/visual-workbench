package com.visualworkbench.shared.assistedselection.backend

import com.visualworkbench.shared.AssistedSourcePixelsLease
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class LiteRtOwnershipTest {
    @Test fun failedNativeRetirementRetainsExactHandleAndDependenciesUntilRetrySucceeds() = runBlocking {
        val owner=EngineOwner(); val events=mutableListOf<String>(); var failing=true
        owner.own(AutoCloseable { events += "environment" })
        owner.own(AutoCloseable { events += "model" })
        val buffer=owner.own(AutoCloseable { events += "buffer"; if (failing) throw IllegalStateException("retained") })
        owner.image=FloatArray(4); owner.embedding=FloatArray(4)
        try { owner.close(); fail("Retirement failure hidden") } catch (_: IllegalStateException) { }
        assertSame(buffer,owner.handles.last()); assertEquals(3,owner.handles.size)
        assertEquals(listOf("buffer"),events); assertNull(owner.image); assertNull(owner.embedding)
        failing=false; owner.close()
        assertEquals(listOf("buffer","buffer","model","environment"),events); assertTrue(owner.handles.isEmpty())
        owner.close(); assertEquals(4,events.size)
    }

    @Test fun failedSourceLeaseCloseStaysReachableAndDoesNotReleaseDependentModels() = runBlocking {
        val owner=EngineOwner(); var failing=true; var calls=0; var nativeClosed=false
        val lease=object : AssistedSourcePixelsLease {
            override val source=backendSource()
            override val rgba=ByteArray(64)
            override suspend fun close() { calls++; if (failing) throw IllegalStateException("source retained") }
        }
        owner.lease=lease; owner.own(AutoCloseable { nativeClosed=true })
        try { owner.close(); fail("Lease failure hidden") } catch (_: IllegalStateException) { }
        assertSame(lease,owner.lease); assertFalse(nativeClosed)
        failing=false; owner.close()
        assertEquals(2,calls); assertNull(owner.lease); assertTrue(nativeClosed)
    }

    @Test fun encoderOnlyRetirementKeepsEnvironmentAndEmbeddingAndRetainsAnyFailedChild() = runBlocking {
        val owner=EngineOwner(); val events=mutableListOf<String>(); var fail=true
        owner.own(AutoCloseable { events += "environment" }); val mark=owner.handles.size
        owner.embedding=FloatArray(4) { 7f }
        owner.own(AutoCloseable { events += "encoder" })
        val buffer=owner.own(AutoCloseable { events += "input"; if (fail) throw IllegalStateException("retained") })
        try { owner.closeTo(mark); fail("Encoder retirement failure hidden") } catch (_: IllegalStateException) { }
        assertSame(buffer,owner.handles.last()); assertEquals(7f,owner.embedding!![0],0f)
        fail=false; owner.closeTo(mark)
        assertEquals(1,owner.handles.size); assertEquals(7f,owner.embedding!![0],0f)
        assertEquals(listOf("input","input","encoder"),events)
        owner.close(); assertEquals("environment",events.last())
    }

    @Test fun callerCancellationJoinsActualAffineWorkAndCloseJoinsTheDedicatedThread() = runBlocking {
        val worker=AffineWorker(); val entered=CompletableDeferred<Unit>(); val release=CompletableDeferred<Unit>()
        var task: Job? = null; var thread: Thread? = null
        try {
            task=launch {
                val caller=currentCoroutineContext()
                worker.run(caller) { thread=Thread.currentThread(); entered.complete(Unit); release.await() }
            }
            entered.await(); task.cancel(); yield(); assertFalse(task.isCompleted)
            assertTrue(thread!!.isAlive); release.complete(Unit); withTimeout(3000) { task.join() }
            assertTrue(task.isCancelled)
            worker.joinClose(); assertFalse(requireNotNull(thread).isAlive)
        } finally { release.complete(Unit); withContext(NonCancellable) { task?.join(); worker.joinClose() } }
    }
}
