package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import java.lang.reflect.Proxy
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
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

/** Going to the background asks the core to write the checkpoint closing
 * would write, through the call the editor's onBackground makes. The project
 * is never closed and nothing else is asked of it. */
class BackgroundSettleTest {
    @Test fun goingToTheBackgroundAsksTheOpenProjectToSettleOnceTheEditorIsIdle(): Unit = runBlocking {
        val project = Settling()
        // A save is in flight: the editor is not idle yet.
        val idle = CompletableDeferred<Boolean>()
        val settle = BackgroundSettle(this) { idle.await() }
        val asking = checkNotNull(settle.background(project) { project })
        yield()
        assertEquals(0, project.asked.get())
        idle.complete(true)
        asking.join()
        assertEquals(1, project.asked.get())
        // The next time the app leaves the screen it is asked again.
        checkNotNull(settle.background(project) { project }).join()
        assertEquals(2, project.asked.get())
    }

    @Test fun nothingIsAskedWithoutAnOpenProjectThatOffersIt(): Unit = runBlocking {
        val settle = BackgroundSettle(this) { true }
        assertNull(settle.background(null) { null })
        // A project without the call (a test double, another platform's) is left alone.
        assertNull(settle.background(untouched()) { null })
    }

    @Test fun aProjectClosedOrReplacedBeforeTheEditorIsIdleIsNotAsked(): Unit = runBlocking {
        val project = Settling()
        val replaced = Settling()
        checkNotNull(BackgroundSettle(this) { true }.background(project) { replaced }).join()
        checkNotNull(BackgroundSettle(this) { true }.background(project) { null }).join()
        // The editor is being disposed: readiness answers false.
        checkNotNull(BackgroundSettle(this) { false }.background(project) { project }).join()
        assertEquals(0, project.asked.get())
        assertEquals(0, replaced.asked.get())
    }

    @Test fun oneRequestAtATimeAndNoneOnceTheAppIsBack(): Unit = runBlocking {
        val project = Settling()
        val idle = CompletableDeferred<Boolean>()
        val settle = BackgroundSettle(this) { idle.await() }
        val first = checkNotNull(settle.background(project) { project })
        // Paused again before the first was answered: nothing more is queued.
        assertNull(settle.background(project) { project })
        // Back on screen while it still waits: it is dropped, so that the
        // checkpoint does not pause the user's work.
        settle.foreground()
        idle.complete(true)
        first.join()
        assertTrue(first.isCancelled)
        assertEquals(0, project.asked.get())
    }

    @Test fun aRefusalIsNoFailureOfTheEditor(): Unit = runBlocking {
        // Drawing again, closed meanwhile, or not written: the log holds every
        // edit. None of it reaches the editor's scope (runBlocking would fail).
        for (kind in listOf(CoreFailureKind.Backpressure, CoreFailureKind.Closed, CoreFailureKind.Storage)) {
            val project = Settling { throw CoreFailure(kind) }
            val asking = checkNotNull(BackgroundSettle(this) { true }.background(project) { project })
            asking.join()
            assertFalse(asking.isCancelled)
            assertEquals(1, project.asked.get())
        }
    }
}
