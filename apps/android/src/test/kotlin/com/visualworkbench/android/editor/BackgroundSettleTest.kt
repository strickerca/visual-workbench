package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import java.lang.reflect.Proxy
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.yield
import org.junit.Assert.*
import org.junit.Test

/** A project none of whose ordinary calls may be made. */
private fun untouched(): WorkbenchProject = Proxy.newProxyInstance(WorkbenchProject::class.java.classLoader,
    arrayOf(WorkbenchProject::class.java)) { _, method, _ -> error("unexpected project call ${method.name}") } as WorkbenchProject

/** A project that offers to settle and counts how often it was asked. */
private class Settling(val answer: suspend () -> Boolean = { true }) : WorkbenchProject by untouched(), WorkbenchSettling {
    val asked = AtomicInteger()
    override suspend fun settle(): Boolean { asked.incrementAndGet(); return answer() }
}

/** Virtual time: a wait ends only when the test moves the clock past it. */
private class VirtualClock {
    var now = 0L
        private set
    private val sleepers = mutableListOf<Pair<Long, CompletableDeferred<Unit>>>()
    suspend fun wait(ms: Long) {
        val woken = CompletableDeferred<Unit>()
        sleepers += (now + ms) to woken
        woken.await()
    }
    /** Lets what was started run up to its next wait, then moves the clock. */
    suspend fun advance(ms: Long) {
        yield()
        now += ms
        sleepers.filter { it.first <= now }.forEach { sleepers.remove(it); it.second.complete(Unit) }
        yield()
    }
}

/** Once the app is out of sight, the core is asked to write the checkpoint
 * closing would write, through the calls the editor's onStop and onStart
 * make. The project is never closed and nothing else is asked of it. */
class BackgroundSettleTest {
    private fun CoroutineScope.settle(clock: VirtualClock, idle: suspend () -> Boolean = { true }) =
        BackgroundSettle(this, idle, clock::wait)

    @Test fun outOfSightTheOpenProjectIsAskedToSettleAfterTheGracePeriod(): Unit = runBlocking {
        val clock = VirtualClock(); val project = Settling()
        val settle = settle(clock)
        val asking = checkNotNull(settle.hidden(project) { project })
        clock.advance(BackgroundSettle.GRACE_MS - 1)
        assertEquals(0, project.asked.get())
        clock.advance(1)
        asking.join()
        assertEquals(2_000L, BackgroundSettle.GRACE_MS)
        assertEquals(1, project.asked.get())
        // The next time the app is out of sight it is asked again.
        val again = checkNotNull(settle.hidden(project) { project })
        clock.advance(BackgroundSettle.GRACE_MS)
        again.join()
        assertEquals(2, project.asked.get())
    }

    @Test fun backInSightWithinTheGracePeriodNothingIsAsked(): Unit = runBlocking {
        // A quick look at another app or at Recents.
        val clock = VirtualClock(); val project = Settling()
        val settle = settle(clock)
        val asking = checkNotNull(settle.hidden(project) { project })
        clock.advance(1_500)
        settle.shown()
        clock.advance(60_000)
        asking.join()
        assertTrue(asking.isCancelled)
        assertEquals(0, project.asked.get())
    }

    @Test fun itWaitsForTheEditorToBeIdleOneRequestAtATimeAndNoneOnceBack(): Unit = runBlocking {
        val clock = VirtualClock(); val project = Settling()
        // An AI run or a save is still in flight once the grace period is over.
        val idle = CompletableDeferred<Boolean>()
        val settle = settle(clock) { idle.await() }
        val first = checkNotNull(settle.hidden(project) { project })
        clock.advance(BackgroundSettle.GRACE_MS)
        assertEquals(0, project.asked.get())
        // Stopped again before the first was answered: nothing more is queued.
        assertNull(settle.hidden(project) { project })
        // Back in sight while it still waits: it is dropped, so that the
        // checkpoint does not pause the user's work.
        settle.shown()
        idle.complete(true)
        first.join()
        assertTrue(first.isCancelled)
        assertEquals(0, project.asked.get())
        // Idle by the end of the next grace period: asked then.
        val next = checkNotNull(settle.hidden(project) { project })
        clock.advance(BackgroundSettle.GRACE_MS)
        next.join()
        assertEquals(1, project.asked.get())
    }

    @Test fun nothingIsAskedWithoutAnOpenProjectThatOffersIt(): Unit = runBlocking {
        val settle = settle(VirtualClock())
        assertNull(settle.hidden(null) { null })
        // A project without the call (a test double, another platform's) is left alone.
        assertNull(settle.hidden(untouched()) { null })
    }

    @Test fun aProjectClosedOrReplacedMeanwhileIsNotAsked(): Unit = runBlocking {
        val clock = VirtualClock(); val project = Settling(); val replaced = Settling()
        val jobs = listOf(
            checkNotNull(settle(clock).hidden(project) { replaced }),
            checkNotNull(settle(clock).hidden(project) { null }),
            // The editor is being disposed: readiness answers false.
            checkNotNull(settle(clock) { false }.hidden(project) { project }))
        clock.advance(BackgroundSettle.GRACE_MS)
        jobs.forEach { it.join() }
        assertEquals(0, project.asked.get())
        assertEquals(0, replaced.asked.get())
    }

    @Test fun aRefusalIsNoFailureOfTheEditor(): Unit = runBlocking {
        // Drawing again, closed meanwhile, or not written: the log holds every
        // edit. None of it reaches the editor's scope (runBlocking would fail).
        for (kind in listOf(CoreFailureKind.Backpressure, CoreFailureKind.Closed, CoreFailureKind.Storage)) {
            val clock = VirtualClock()
            val project = Settling { throw CoreFailure(kind) }
            val asking = checkNotNull(settle(clock).hidden(project) { project })
            clock.advance(BackgroundSettle.GRACE_MS)
            asking.join()
            assertFalse(asking.isCancelled)
            assertEquals(1, project.asked.get())
        }
    }
}
