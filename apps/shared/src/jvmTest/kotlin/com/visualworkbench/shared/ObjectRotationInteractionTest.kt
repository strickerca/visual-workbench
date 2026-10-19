@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

class ObjectRotationInteractionTest {
    @Test fun manySamplesAndTwoTargetsFinishAsOneAbsoluteCommitWithTheRealUpSnap() = runBlocking {
        val f = Fixture(this)
        try {
            val token = f.begin()!!
            repeat(80) { assertTrue(f.controls.move(token, Point(it.toDouble(), 1.0), it % 2 == 0)) }
            assertTrue(f.requests.isEmpty())
            assertTrue(f.controls.finish(token, Point(29.0, 1.0), true))
            assertFalse(f.controls.finish(token, Point(40.0, 1.0), false))
            f.idle()
            val request = f.requests.single()
            assertEquals(token.gestureId, request.gestureId)
            assertEquals(token.frame, request.frame)
            assertEquals(listOf(15.0, 15.0), request.commands.map { (it as EditCommand.SetTransform).transform.e })
            assertTrue(f.solverInputs.all { it == listOf(Transform(e = 4.0), Transform(e = 8.0)) })
            assertTrue(f.snaps.last())
            assertFalse(f.controls.state.value.busy)
            assertTrue(f.controls.state.value.preview.isEmpty())
        } finally { f.close() }
    }

    @Test fun identityAndCancelledContactNeverEnterTheEditCallback() = runBlocking {
        val f = Fixture(this)
        try {
            assertTrue(f.controls.finish(f.begin()!!, f.first, false))
            val token = f.begin()!!
            assertTrue(f.controls.move(token, Point(20.0, 1.0), false))
            f.controls.cancel()
            assertFalse(f.controls.finish(token, Point(30.0, 1.0), false))
            f.idle(); assertTrue(f.requests.isEmpty())
        } finally { f.close() }
    }

    @Test fun queuedCommitRechecksProjectEpochVisibleHashCameraPageAndSelection() = runBlocking {
        for (change in listOf<(Fixture) -> Unit>(
            { it.attachment = it.attachment.copy(project = Project()) },
            { it.attachment = it.attachment.copy(epoch = 9) },
            { it.document { d -> d.copy(render = d.render.copy(revision = d.render.revision.copy(stateHash = "b".repeat(64)))) } },
            { it.attachment = it.attachment.copy(camera = it.attachment.camera.copy(rotation = 0.3)) },
            { it.document { d -> d.copy(page = 1u) } },
            { it.attachment = it.attachment.copy(selectedObjectIds = setOf("one")) },
        )) {
            val f = Fixture(this)
            try {
                f.mutations.lock()
                assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false))
                change(f)
                f.mutations.unlock(); f.idle()
                assertTrue(f.requests.isEmpty())
                assertEquals(ObjectRotationFailure.Stale, f.controls.state.value.failure)
            } finally { if (f.mutations.isLocked) f.mutations.unlock(); f.close() }
        }
    }

    @Test fun staleMoveSealsItsContactAndOldTokenCannotAffectANewContact() = runBlocking {
        val f = Fixture(this)
        try {
            val old = f.begin()!!
            f.attachment = f.attachment.copy(epoch = 2)
            assertFalse(f.controls.move(old, Point(20.0, 1.0), false))
            val fresh = f.begin()!!
            assertFalse(f.controls.move(old, Point(30.0, 1.0), false))
            assertTrue(f.controls.move(fresh, Point(40.0, 1.0), false))
            assertEquals(40.0, f.controls.state.value.preview["one"]!!.e, 0.0)
        } finally { f.close() }
    }

    @Test fun cancellationRetainsTheNativeProducerUntilDetachJoinsIt() = runBlocking {
        val f = Fixture(this)
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var detached: Deferred<Unit>? = null
        try {
            f.commitGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() }; currentCoroutineContext().ensureActive() }
            assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false)); entered.await()
            f.controls.cancel()
            assertTrue(f.controls.state.value.busy)
            assertNull(f.begin())
            detached = async(start = CoroutineStart.UNDISPATCHED) { f.controls.detach() }
            assertFalse(detached.isCompleted)
            release.complete(Unit)
            withTimeout(3000) { detached.await() }
            assertFalse(f.controls.state.value.busy)
            assertEquals(1, f.requests.size)
        } finally { release.complete(Unit); withContext(NonCancellable) { detached?.join(); f.close() } }
    }

    @Test fun waitingForMutexCancellationHasNoCommitAndDoesNotRetainAnUncertainEdit() = runBlocking {
        val f = Fixture(this)
        try {
            f.mutations.lock()
            assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false))
            f.controls.cancel(); f.idle()
            assertTrue(f.requests.isEmpty())
            assertEquals(ObjectRotationFailure.Cancelled, f.controls.state.value.failure)
            assertNull(f.controls.state.value.preview["one"])
        } finally { if (f.mutations.isLocked) f.mutations.unlock(); f.close() }
    }

    @Test fun uncertainFailureNeverAutomaticallyMakesAnotherUndoTransaction() = runBlocking {
        val f = Fixture(this)
        try {
            f.commitGate = { throw CoreFailure(CoreFailureKind.Storage) }
            assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false)); f.idle()
            assertEquals(ObjectRotationFailure.Unconfirmed, f.controls.state.value.failure)
            f.controls.cancel()
            assertNull(f.begin()); assertEquals(1, f.requests.size)
            f.commitGate = {}
            f.controls.reconciled()
            assertNotNull(f.begin())
            assertEquals(1, f.requests.size)
        } finally { f.close() }
    }

    @Test fun refusedCommitAndPreCancelledOrClosedScopesNeverLeaveBusyState() = runBlocking {
        val f = Fixture(this)
        try {
            f.accept = false
            assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false)); f.idle()
            assertEquals(ObjectRotationFailure.Stale, f.controls.state.value.failure)
            assertFalse(f.controls.state.value.busy)
            f.controls.close(); assertNull(f.begin())
        } finally { f.close() }
        val cancelled = CoroutineScope(coroutineContext + Job().apply { cancel() })
        val g = Fixture(cancelled)
        try { assertNull(g.begin()); assertFalse(g.controls.state.value.busy) } finally { g.close() }
    }

    @Test fun synchronousIdCallbackCannotRebindTheContactAndInvalidTargetsNeverCallSolver() = runBlocking {
        val f = Fixture(this)
        try {
            f.onId = { f.attachment = f.attachment.copy(epoch = 99) }
            assertNull(f.begin())
            f.onId = {}
            f.document { d -> d.copy(render = d.render.copy(items = d.render.items.map { it.copy(locked = true) })) }
            f.solverInputs.clear()
            assertNull(f.begin()); assertTrue(f.solverInputs.isEmpty())
        } finally { f.close() }
    }

    @Test fun finalInvalidSampleAndRootGuardRefuseWithoutEnteringCommit() = runBlocking {
        val f = Fixture(this)
        try {
            assertFalse(f.controls.finish(f.begin()!!, Point(Double.NaN, 1.0), false))
            assertEquals(ObjectRotationFailure.Invalid, f.controls.state.value.failure)
            val token = f.begin()!!; f.allowed = false
            assertFalse(f.controls.finish(token, Point(20.0, 1.0), false))
            assertNull(f.begin()); assertTrue(f.requests.isEmpty())
        } finally { f.close() }
    }

    @Test fun reentrantCommitCallbackSeesTheOwnedOperationBeforeItCanStartAnotherGesture() = runBlocking {
        val f = Fixture(this)
        try {
            f.commitGate = { assertNull(f.begin()); f.controls.cancel(); currentCoroutineContext().ensureActive() }
            assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false)); f.idle()
            assertEquals(1, f.requests.size)
            assertEquals(ObjectRotationFailure.Unconfirmed, f.controls.state.value.failure)
        } finally { f.close() }
    }

    @Test fun parentCancellationBeforeTheLazyBodyStartsRetiresBusyWithoutACommit() = runBlocking {
        val owner = Job()
        val f = Fixture(CoroutineScope(coroutineContext + owner))
        try {
            assertTrue(f.controls.finish(f.begin()!!, Point(20.0, 1.0), false))
            owner.cancel(); owner.join()
            assertFalse(f.controls.state.value.busy)
            assertEquals(ObjectRotationFailure.Cancelled, f.controls.state.value.failure)
            assertTrue(f.requests.isEmpty())
        } finally { f.close() }
    }

    @Test fun cancellingParentAfterContactStartFencesFurtherPreviewSamples() = runBlocking {
        val owner = Job()
        val f = Fixture(CoroutineScope(coroutineContext + owner))
        try {
            val token = f.begin()!!
            assertTrue(f.controls.move(token, Point(20.0, 1.0), false))
            val calls = f.solverInputs.size
            owner.cancel(); owner.join()
            assertFalse(f.controls.move(token, Point(30.0, 1.0), false))
            assertEquals(calls, f.solverInputs.size)
            assertFalse(f.controls.state.value.active)
            assertTrue(f.controls.state.value.preview.isEmpty())
            assertTrue(f.requests.isEmpty())
        } finally { f.close() }
    }

    private class Fixture(parent: CoroutineScope) {
        val mutations = Mutex()
        val first = Point(0.0, -1.0)
        val requests = mutableListOf<ObjectRotationCommit>()
        val solverInputs = mutableListOf<List<Transform>>()
        val snaps = mutableListOf<Boolean>()
        var commitGate: suspend () -> Unit = {}
        var onId: () -> Unit = {}
        var accept = true
        var allowed = true
        var ids = 0
        private val info = ProjectInfo("project", "title", "device", 1uL, false, false, 1uL, "a".repeat(64), listOf("document"))
        private fun item(id: String, offset: Double) = RenderItem(id, "layer", 1.0, "normal", Rect(0.0, 0.0, 10.0, 10.0),
            Contours(longArrayOf(), longArrayOf(), uintArrayOf()), Transform(e = offset), ObjectStyle(0u, 1.0),
            Shape.Rectangle(Rect(0.0, 0.0, 10.0, 10.0)), false)
        var attachment = ObjectRotationAttachment(Project(), DocumentSnapshot("document", "title", 100u, 100u, 8u,
            emptyList(), RenderList(info, listOf(item("one", 4.0), item("two", 8.0)))), 1,
            Camera(Point(0.0, 0.0), 1.0, 0.0, 100.0, 100.0), setOf("one", "two"))
        val controls = ObjectRotationInteraction(parent, mutations, { onId(); "gesture-${++ids}" },
            { values, _, start, point, snap ->
                solverInputs += values.toList(); snaps += snap
                if (point == start) values else values.map { it.copy(e = if (snap) 15.0 else point.x) }
            }, { attachment }, { allowed }, { request -> requests += request; commitGate(); accept })
        fun document(change: (DocumentSnapshot) -> DocumentSnapshot) { attachment = attachment.copy(document = change(attachment.document)) }
        fun begin() = controls.begin(Point(0.0, 0.0), first)
        suspend fun idle() { withTimeout(3000) { while (controls.state.value.busy) yield() } }
        suspend fun close() { controls.close() }
    }

    private class Project : WorkbenchProject {
        override val changes = emptyFlow<ProjectChange>()
        override suspend fun info(): ProjectInfo = error("not used")
        override suspend fun document(documentId: String): DocumentSnapshot = error("not used")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("not used")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("not used")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("root callback owns edit")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("not used")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("not used")
        override suspend fun export(options: ExportOptions): ExportResult = error("not used")
        override suspend fun close() = Unit
    }
}
