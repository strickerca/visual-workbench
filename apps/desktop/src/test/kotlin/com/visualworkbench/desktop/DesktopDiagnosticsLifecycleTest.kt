package com.visualworkbench.desktop

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class DesktopDiagnosticsLifecycleTest {
    private class Exporter {
        var held = false
        var retires = true
        var requests = 0
        val lifecycle = DesktopDiagnosticsLifecycle(
            held = { held },
            requestClose = { requests++; if (retires) held = false; !held },
            closeWithin = { if (retires) held = false; !held },
        )
    }

    @Test fun repeatedDiagnosticsCommandKeepsTheExactFirstPreview() {
        val exporter = Exporter()
        var prepared = 0
        var applied = 0
        fun enter() = exporter.lifecycle.changeRoute(true, false,
            prepare = { prepared++; exporter.held = true }, apply = { applied++ })
        assertTrue(enter())
        val route = exporter.lifecycle.route()
        assertFalse(enter())
        assertEquals(1, prepared)
        assertEquals(1, applied)
        assertEquals(0, exporter.requests)
        assertSame(route, exporter.lifecycle.route())
    }

    @Test fun pendingCloseKeepsTheRouteAndBlocksCompetingCommandsUntilRetry() {
        val exporter = Exporter().apply { held = true; retires = false }
        val route = exporter.lifecycle.route()
        var applied = false
        assertFalse(exporter.lifecycle.changeRoute(false, false, {}, { applied = true }))
        assertFalse(applied)
        assertTrue(exporter.held)
        assertSame(route, exporter.lifecycle.route())
        assertFalse(exporter.lifecycle.allowsCommand(false))
        assertTrue(exporter.lifecycle.allowsCommand(false, diagnostics = true))
        assertTrue(exporter.lifecycle.allowsCommand(false, quit = true))
        exporter.retires = true
        assertTrue(exporter.lifecycle.changeRoute(false, false, {}, { applied = true }))
        assertTrue(applied)
        assertFalse(exporter.held)
        assertNotSame(route, exporter.lifecycle.route())
        assertTrue(exporter.lifecycle.allowsCommand(false))
    }

    @Test fun chooserCallbackCannotRebindAfterLeavingAndReenteringDiagnostics() {
        val exporter = Exporter()
        exporter.lifecycle.changeRoute(true, false, { exporter.held = true }, {})
        val original = exporter.lifecycle.route()
        assertTrue(exporter.lifecycle.mayChoose(original, false, true, true, false))
        assertTrue(exporter.lifecycle.changeRoute(false, false, {}, {}))
        assertTrue(exporter.lifecycle.changeRoute(true, false, { exporter.held = true }, {}))
        assertFalse(exporter.lifecycle.mayChoose(original, false, true, true, false))
        assertTrue(exporter.lifecycle.mayChoose(exporter.lifecycle.route(), false, true, true, false))
    }

    @Test fun chooserRechecksShutdownWindowRouteAndCompetingModalAtCompletion() {
        val exporter = Exporter().apply { held = true }
        val route = exporter.lifecycle.route()
        assertFalse(exporter.lifecycle.mayChoose(route, true, true, true, false))
        assertFalse(exporter.lifecycle.mayChoose(route, false, false, true, false))
        assertFalse(exporter.lifecycle.mayChoose(route, false, true, false, false))
        assertFalse(exporter.lifecycle.mayChoose(route, false, true, true, true))
        assertFalse(exporter.lifecycle.mayChoose(Any(), false, true, true, false))
        exporter.held = false
        assertFalse(exporter.lifecycle.mayChoose(route, false, true, true, false))
    }

    @Test fun pendingShutdownDoesNotSealDraftsOrCloseAnyParent() = runBlocking {
        val exporter = Exporter().apply { held = true; retires = false }
        val calls = mutableListOf<String>()
        assertFalse(exporter.lifecycle.closeParentsAfterRetirement({ calls.add("seal"); true }, { calls.add("close") }))
        assertTrue(calls.isEmpty())
        assertTrue(exporter.held)
        exporter.retires = true
        assertTrue(exporter.lifecycle.closeParentsAfterRetirement({ calls.add("seal"); true }, { calls.add("close") }))
        assertEquals(listOf("seal", "close"), calls)
    }

    @Test fun closeAdmissionIsRecheckedAfterTheActualWait() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        var cleanDraft = true
        var sealed = false
        var closed = false
        val lifecycle = DesktopDiagnosticsLifecycle({ true }, { false }, {
            entered.complete(Unit)
            release.await()
            true
        })
        val closing = async {
            lifecycle.closeParentsAfterRetirement({ sealed = cleanDraft; cleanDraft }, { closed = true })
        }
        entered.await()
        assertFalse(sealed)
        assertFalse(closed)
        cleanDraft = false
        release.complete(Unit)
        assertFalse(closing.await())
        assertFalse(sealed)
        assertFalse(closed)
    }

    @Test fun quittingCannotPrepareReportsChangeRoutesOrAdmitCommands() {
        val exporter = Exporter()
        val route = exporter.lifecycle.route()
        assertFalse(exporter.lifecycle.changeRoute(true, true, { fail("prepared during quit") }, { fail("changed route") }))
        assertSame(route, exporter.lifecycle.route())
        assertFalse(exporter.lifecycle.allowsCommand(true))
        assertFalse(exporter.lifecycle.allowsCommand(true, diagnostics = true))
        assertFalse(exporter.lifecycle.allowsCommand(true, quit = true))
    }
}
