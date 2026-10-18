package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

class DocumentNavigatorTest {
    @Test fun completeTwentyFourDocumentCatalogUsesNoProjectReadsOrSecondManager() {
        val navigator = DocumentNavigator()
        val document = snapshot()
        navigator.bind(document, 7)
        navigator.active(true)
        assertEquals(24, navigator.state.value.documentIds.size)
        val request = request(navigator, id(24))!!
        assertTrue(navigator.matches(request, document, 7))
        assertEquals(id(24), request.documentId)
        assertTrue(navigator.state.value.busy)
        assertNull(request(navigator, id(23)))
        navigator.finish(request)
        assertFalse(navigator.state.value.busy)
    }

    @Test fun copiedCatalogCannotBeChangedByItsProducerAfterBinding() {
        val ids = (1..24).map(::id).toMutableList()
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(info(ids)), 7)
        ids.clear()
        assertEquals(24, navigator.state.value.documentIds.size)
    }

    @Test fun catalogLimitsAndDuplicatesRefuseTheWholeListInsteadOfTruncating() {
        for (ids in listOf(emptyList(), listOf(id(1), id(1)), listOf(id(2)), (1..129).map(::id))) {
            val navigator = DocumentNavigator()
            navigator.bind(snapshot(info(ids)), 7)
            navigator.active(true)
            assertFalse(navigator.state.value.ready)
            assertTrue(navigator.state.value.documentIds.isEmpty())
            assertNotNull(navigator.state.value.issue)
        }
    }

    @Test fun staleEpochAndSameSequenceDifferentHashRefuseOldRequests() {
        val navigator = DocumentNavigator()
        val first = snapshot()
        navigator.bind(first, 7); navigator.active(true)
        val request = request(navigator, id(2))!!
        assertFalse(navigator.matches(request, first, 8))
        val offline = snapshot(info().copy(stateHash = "different-visible-state"))
        assertFalse(navigator.matches(request, offline, 7))
        navigator.bind(offline, 7)
        assertFalse(navigator.matches(request, offline, 7))
        assertNull(navigator.request(request.binding, request.attachmentEpoch, id(3)))
    }

    @Test fun reopenAndRebindOfTheSameDocumentRefuseAStalePanelClick() {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7); navigator.active(true)
        val old = navigator.state.value
        navigator.bind(snapshot(), 8)
        assertNull(navigator.request(old.binding!!, old.attachmentEpoch!!, id(2)))
        assertNotNull(request(navigator, id(2)))
    }

    @Test fun cancelledRequestCannotBeResurrectedByARepeatedRequestOrOldCompletion() {
        val navigator = DocumentNavigator()
        val document = snapshot()
        navigator.bind(document, 7); navigator.active(true)
        val old = request(navigator, id(2))!!
        navigator.cancel()
        val next = request(navigator, id(2))!!
        assertFalse(navigator.matches(old, document, 7))
        assertTrue(navigator.matches(next, document, 7))
        navigator.finish(old, "old failure")
        assertTrue(navigator.state.value.busy)
        assertNull(navigator.state.value.issue)
    }

    @Test fun observedChangeInvalidatesBeforeTheNextVisibleSnapshotAndDetachForgetsCatalog() {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7); navigator.active(true)
        val old = request(navigator, id(2))!!
        navigator.invalidate()
        assertFalse(navigator.matches(old, snapshot(), 7))
        assertNull(navigator.request(old.binding, 7, id(3)))
        navigator.bind(snapshot(info().copy(hostSeq = 2uL)), 7)
        assertTrue(navigator.state.value.ready)
        navigator.detach()
        assertEquals(DocumentNavigationState(), navigator.state.value)
    }

    @Test fun hiddenChooserCurrentAndUnknownDocumentCannotIssueSwitches() {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7)
        assertNull(request(navigator, id(2)))
        navigator.active(true)
        assertNull(request(navigator, id(1)))
        assertNull(request(navigator, id(25)))
    }

    @Test fun targetLoadChecksRevisionBeforeAndAfterFetchWithoutWritingHistory() = runBlocking {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7); navigator.active(true)
        val request = request(navigator, id(24))!!
        val project = Project()
        val target = loadNavigationTarget(project, request)
        assertEquals(id(24), target.documentId)
        assertEquals(2, project.infoReads)
        assertEquals(listOf(id(24)), project.documentReads)
        assertTrue(navigator.matches(request, snapshot(), 7))
    }

    @Test fun staleBeforeLoadRefusesBeforeReadingTarget() = runBlocking {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7); navigator.active(true)
        val request = request(navigator, id(2))!!
        val project = Project().also { it.revision = info().copy(stateHash = "offline") }
        stale { loadNavigationTarget(project, request) }
        assertTrue(project.documentReads.isEmpty())
    }

    @Test fun revisionChangeDuringLoadAndWrongTargetAreRefused() = runBlocking {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7); navigator.active(true)
        val request = request(navigator, id(2))!!
        val changed = Project().also { p -> p.afterDocument = { p.revision = info().copy(hostSeq = 2uL) } }
        stale { loadNavigationTarget(changed, request) }
        val wrong = Project().also { it.wrongTarget = true }
        stale { loadNavigationTarget(wrong, request) }
    }

    @Test fun cancellationDuringBorrowedFetchSettlesBeforeRootCanCloseTheProject() = runBlocking {
        val navigator = DocumentNavigator()
        navigator.bind(snapshot(), 7); navigator.active(true)
        val request = request(navigator, id(2))!!
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val project = Project().also { it.pause = { withContext(NonCancellable) { entered.complete(Unit); release.await() } } }
        var adopted = false
        val job = launch { loadNavigationTarget(project, request); adopted = true }
        try {
            entered.await(); job.cancel(); navigator.cancel()
            assertFalse(job.isCompleted)
            release.complete(Unit); withTimeout(3000) { job.join() }
            assertFalse(adopted)
            assertFalse(navigator.matches(request, snapshot(), 7))
        } finally { release.complete(Unit); withContext(NonCancellable) { job.cancelAndJoin() } }
    }

    private suspend fun stale(block: suspend () -> Unit) {
        try { block(); fail("Expected a stale navigation refusal") }
        catch (error: WorkflowFailure) { assertEquals(WorkflowFailureKind.Stale, error.kind) }
    }

    private class Project : WorkbenchProject {
        var revision = DocumentNavigatorTest.info()
        var infoReads = 0
        val documentReads = mutableListOf<String>()
        var afterDocument: () -> Unit = {}
        var pause: suspend () -> Unit = {}
        var wrongTarget = false
        override val changes = emptyFlow<ProjectChange>()
        override suspend fun info(): ProjectInfo { infoReads++; return revision }
        override suspend fun document(documentId: String): DocumentSnapshot {
            documentReads += documentId
            val result = snapshot(revision, if (wrongTarget) id(1) else documentId)
            pause(); afterDocument(); return result
        }
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Navigation cannot edit history")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Navigation cannot edit history")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Existing root owns rendering")
        override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
        override suspend fun close(): Unit = error("Existing root owns the project")
    }

    private companion object {
        fun id(n: Int) = "00000000-0000-7000-8000-${n.toString().padStart(12, '0')}"
        fun info(ids: List<String> = (1..24).map(::id)) = ProjectInfo(id(100), "Project", "local-device", 1uL, false, false, 1uL, "visible-state", ids)
        fun snapshot(info: ProjectInfo = info(), document: String = id(1)) = DocumentSnapshot(document, "Document", 100u, 100u, 8u, emptyList(), RenderList(info, emptyList()))
        fun request(navigator: DocumentNavigator, id: String): DocumentNavigationRequest? {
            val state = navigator.state.value
            return state.binding?.let { navigator.request(it, state.attachmentEpoch!!, id) }
        }
    }
}
