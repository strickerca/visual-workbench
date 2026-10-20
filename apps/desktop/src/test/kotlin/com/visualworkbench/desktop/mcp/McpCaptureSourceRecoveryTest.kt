package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Assume.assumeNoException
import org.junit.Test
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.util.Comparator

class McpCaptureSourceRecoveryTest {
    private val binding = WorkflowBinding("00000000-0000-7000-8000-000000000001",
        "00000000-0000-7000-8000-000000000002", 7uL, "c".repeat(64))
    private val expected = PackageInfo("00000000-0000-7000-8000-000000000003", "generic", null,
        binding, "d".repeat(64), "e".repeat(64), "2026-10-18", 1024uL, 1u, emptyList(), true, false)
    private fun info() = ProjectInfo(binding.projectId, "Fixture", "local", 10uL, false, false,
        binding.hostSeq, binding.stateHash, listOf(binding.documentId))

    private class Fixture : AutoCloseable {
        val parent = Files.createTempDirectory("vw-mcp-source-recovery-").toRealPath()
        val root: Path = Files.createDirectory(parent.resolve("mcp-captures-v1"))
        fun project(number: Int, owner: ByteArray? = "Visual Workbench private MCP capture v1\n".toByteArray(),
            complete: Boolean = true): Path {
            val name = "00000000-0000-7000-8000-${number.toString().padStart(12, '0')}"
            val work = Files.createDirectory(root.resolve(name))
            if (owner != null) Files.write(work.resolve("owner-v1"), owner)
            val project = work.resolve("project.vwb")
            if (complete) Files.createDirectory(project)
            return project
        }
        override fun close() {
            // Owned temporary fixtures only; walk does not follow fixture links.
            Files.walk(parent).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) } }
        }
    }

    private class Core(private val opening: suspend (Path) -> WorkbenchProject) : WorkbenchCore {
        val opened = mutableListOf<Path>()
        override suspend fun open(path: String): WorkbenchProject {
            val candidate = Path.of(path); opened.add(candidate)
            return opening(candidate)
        }
        override fun newId(unixMs: ULong): String = error("Unused")
        override fun newDeviceId(): String = error("Unused")
        override suspend fun create(options: CreateProject): WorkbenchProject = error("Unused")
        override suspend fun layoutText(text: String, font: String, size: Float): TextLayout = error("Unused")
        override fun cameraMatrix(camera: Camera, inverse: Boolean): Transform = error("Unused")
        override fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>): List<Point> = error("Unused")
    }

    private class Project(
        private val reading: suspend () -> ProjectInfo, private val closing: suspend () -> Unit = {},
    ) : WorkbenchProject {
        var closes = 0; var closeFinished = false
        override val changes = emptyFlow<ProjectChange>()
        override suspend fun info(): ProjectInfo = reading()
        override suspend fun close() { closes++; closing(); closeFinished = true }
        override suspend fun document(documentId: String): DocumentSnapshot = error("Unused")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("Unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("Unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("Unused")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("Unused")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("Unused")
        override suspend fun export(options: ExportOptions): ExportResult = error("Unused")
    }

    private suspend fun refused(body: suspend () -> Any?) {
        try { body(); fail("Unsafe inventory was admitted") } catch (_: IllegalArgumentException) { }
    }
    private suspend fun failed(expected: Throwable, body: suspend () -> Any?) {
        try { body(); fail("Failure was swallowed") } catch (actual: Throwable) {
            assertEquals(expected.javaClass, actual.javaClass)
            // A recovered copy must identify this exact retained original.
            assertSame(expected, if (actual === expected) actual else actual.cause)
        }
    }
    private fun symlink(link: Path, target: Path) {
        try { Files.createSymbolicLink(link, target) }
        catch (error: UnsupportedOperationException) { assumeNoException(error) }
        catch (error: IOException) { assumeNoException(error) }
        catch (error: SecurityException) { assumeNoException(error) }
    }

    @Test fun findsAnExistingCanonicalProjectAndClosesBeforeReturning() = runBlocking {
        Fixture().use { fixture ->
            val candidate = fixture.project(1)
            val project = Project({ info() })
            val core = Core { project }
            assertEquals(candidate, findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()))
            assertEquals(listOf(candidate), core.opened)
            assertEquals(1, project.closes); assertTrue(project.closeFinished)
        }
    }

    @Test fun exactProjectDocumentSequenceAndHashMustAllMatch() = runBlocking {
        Fixture().use { fixture ->
            val values = listOf(info().copy(projectId = "other-project"), info().copy(documentIds = listOf("other-document")),
                info().copy(hostSeq = binding.hostSeq + 1uL), info().copy(stateHash = "f".repeat(64)), info())
            val candidates = values.indices.map { fixture.project(it + 1) }
            val projects = values.map { value -> Project({ value }) }
            val core = Core { path ->
                val index = candidates.indexOf(path)
                if (index > 0) assertTrue(projects[index - 1].closeFinished)
                projects[index]
            }
            assertEquals(candidates.last(), findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()))
            assertEquals(candidates, core.opened)
            assertTrue(projects.all { it.closes == 1 && it.closeFinished })
        }
    }

    @Test fun mismatchingProjectReturnsNullAfterClosingAndExcludedPathsAreNotOpened() = runBlocking {
        Fixture().use { fixture ->
            val known = fixture.project(1); val unknown = fixture.project(2)
            val project = Project({ info().copy(hostSeq = 999uL) })
            val core = Core { path -> assertEquals(unknown, path); project }
            assertNull(findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement(), setOf(known)))
            assertEquals(listOf(unknown), core.opened)
            assertTrue(project.closeFinished)
        }
    }

    @Test fun missingRootAndOwnedInterruptedCaptureDoNotOpenAProject() = runBlocking {
        Fixture().use { fixture ->
            val core = Core { error("No complete project exists") }
            fixture.project(1, complete = false)
            assertNull(findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()))
            val missing = fixture.parent.resolve("missing").resolve("mcp-captures-v1")
            assertNull(findRetainedMcpCaptureProject(missing, expected, core, McpCaptureProjectRetirement()))
            assertTrue(core.opened.isEmpty())
        }
    }

    @Test fun missingWrongOversizeOrForeignOwnerRejectsEntireInventoryBeforeOpen() = runBlocking {
        val marker = "Visual Workbench private MCP capture v1\n".toByteArray()
        val wrong = marker.copyOf().also { it[0] = 0 }
        for (owner in listOf(null, ByteArray(4096), marker.copyOf(marker.size - 1), wrong)) {
            Fixture().use { fixture ->
                fixture.project(1); fixture.project(2, owner)
                val core = Core { error("Inventory must settle before open") }
                refused { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
                assertTrue(core.opened.isEmpty())
            }
        }
    }

    @Test fun foreignDirectEntriesAndProjectFilesAreRejectedBeforeAnyOpen() = runBlocking {
        Fixture().use { fixture ->
            fixture.project(1)
            Files.write(fixture.root.resolve("foreign.txt"), byteArrayOf(1))
            val core = Core { error("Foreign entry must refuse") }
            refused { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            assertTrue(core.opened.isEmpty())
        }
        Fixture().use { fixture ->
            fixture.project(1)
            val invalid = fixture.project(2, complete = false)
            Files.write(invalid, byteArrayOf(1))
            val core = Core { error("Project must be a directory") }
            refused { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            assertTrue(core.opened.isEmpty())
        }
    }

    @Test fun alternateNoncanonicalAndArbitraryRootPathsAreRejected() = runBlocking {
        Fixture().use { fixture ->
            val core = Core { error("Alternate root must refuse") }
            for (root in listOf(Path.of("mcp-captures-v1"), fixture.parent,
                fixture.root.resolve("..").resolve("mcp-captures-v1"))) {
                refused { findRetainedMcpCaptureProject(root, expected, core, McpCaptureProjectRetirement()) }
            }
            assertTrue(core.opened.isEmpty())
        }
    }

    @Test fun linkedRootAncestorWorkdirMarkerAndProjectCannotEscapeThePrivateInventory() = runBlocking {
        Fixture().use { fixture ->
            fixture.project(1)
            val alias = fixture.parent.resolve("alias")
            symlink(alias, fixture.parent)
            val core = Core { error("Linked root ancestor must refuse") }
            refused { findRetainedMcpCaptureProject(alias.resolve("mcp-captures-v1"), expected, core, McpCaptureProjectRetirement()) }
            assertTrue(core.opened.isEmpty())
        }
        for (kind in listOf("workdir", "owner", "project")) {
            Fixture().use { fixture ->
                val outside = Files.createDirectory(fixture.parent.resolve("outside"))
                val candidate = fixture.project(1)
                when (kind) {
                    "workdir" -> {
                        Files.delete(candidate); Files.delete(candidate.parent.resolve("owner-v1")); Files.delete(candidate.parent)
                        symlink(candidate.parent, outside)
                    }
                    "owner" -> {
                        val marker = candidate.parent.resolve("owner-v1")
                        Files.move(marker, outside.resolve("owner-v1")); symlink(marker, outside.resolve("owner-v1"))
                    }
                    else -> { Files.delete(candidate); symlink(candidate, outside) }
                }
                val core = Core { error("Linked entry must refuse") }
                refused { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
                assertTrue(core.opened.isEmpty())
            }
        }
    }

    @Test fun thirtyTwoCandidatesRemainSequentialAndAThirtyThirdRefusesBeforeAnyOpen() = runBlocking {
        Fixture().use { fixture ->
            val candidates = (1..32).map { fixture.project(it) }
            val projects = candidates.indices.map { index -> Project({ if (index == 31) info() else info().copy(hostSeq = 999uL) }) }
            val core = Core { path ->
                val index = candidates.indexOf(path)
                if (index > 0) assertTrue(projects[index - 1].closeFinished)
                projects[index]
            }
            assertEquals(candidates.last(), findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()))
            assertEquals(32, core.opened.size)
            assertTrue(projects.all { it.closes == 1 && it.closeFinished })
        }
        Fixture().use { fixture ->
            repeat(33) { fixture.project(it + 1) }
            val core = Core { error("Over-limit inventory must refuse before open") }
            refused { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            assertTrue(core.opened.isEmpty())
        }
    }

    @Test fun unusableUnknownCandidateOpenMayBeSkippedWithoutOpeningInParallel() = runBlocking {
        Fixture().use { fixture ->
            val candidates = (1..4).map { fixture.project(it) }
            val kinds = listOf(CoreFailureKind.Invalid, CoreFailureKind.Storage, CoreFailureKind.Unsupported)
            val project = Project({ info() })
            val core = Core { path ->
                val index = candidates.indexOf(path)
                if (index < kinds.size) throw CoreFailure(kinds[index])
                project
            }
            assertEquals(candidates.last(), findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()))
            assertEquals(candidates, core.opened)
            assertEquals(1, project.closes)
        }
    }

    @Test fun openCancellationClosedCapacityWorkerAndUnexpectedFailuresPropagate() = runBlocking {
        val errors = listOf(CancellationException("Fixture cancellation"), CoreFailure(CoreFailureKind.Cancelled),
            CoreFailure(CoreFailureKind.Closed), CoreFailure(CoreFailureKind.Backpressure), CoreFailure(CoreFailureKind.Worker),
            IllegalStateException("Fixture unknown open failure"))
        for (error in errors) Fixture().use { fixture ->
            val first = fixture.project(1); fixture.project(2)
            val core = Core { throw error }
            failed(error) { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            assertEquals(listOf(first), core.opened)
        }
    }

    @Test fun infoFailureClosesAcquiredCandidateAndStopsTheScan() = runBlocking {
        Fixture().use { fixture ->
            val first = fixture.project(1); fixture.project(2)
            val error = IllegalStateException("Fixture info failure")
            val project = Project({ throw error })
            val core = Core { project }
            failed(error) { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            assertEquals(listOf(first), core.opened)
            assertEquals(1, project.closes); assertTrue(project.closeFinished)
        }
    }

    @Test fun closeIsJoinedBeforeOpeningTheNextCandidate() = runBlocking {
        Fixture().use { fixture ->
            val first = fixture.project(1); val second = fixture.project(2)
            val entered = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
            val before = Project({ info().copy(hostSeq = 999uL) }, { entered.complete(Unit); finish.await() })
            val after = Project({ info() })
            val core = Core { path -> if (path == first) before else { assertTrue(before.closeFinished); after } }
            val finding = async { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            try {
                withTimeout(5000) { entered.await() }
                assertFalse(finding.isCompleted); assertEquals(listOf(first), core.opened)
                finish.complete(Unit)
                assertEquals(second, withTimeout(5000) { finding.await() })
                assertTrue(before.closeFinished && after.closeFinished)
            } finally { finish.complete(Unit); finding.cancelAndJoin() }
        }
    }

    @Test fun cancellationDuringInfoStillJoinsNonCancellableCloseBeforeReturning() = runBlocking {
        Fixture().use { fixture ->
            val first = fixture.project(1); fixture.project(2)
            val reading = CompletableDeferred<Unit>(); val closing = CompletableDeferred<Unit>(); val finish = CompletableDeferred<Unit>()
            val project = Project({ reading.complete(Unit); awaitCancellation() }, {
                assertTrue(currentCoroutineContext().isActive); closing.complete(Unit); finish.await()
            })
            val core = Core { project }
            val finding = async { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            try {
                withTimeout(5000) { reading.await() }; finding.cancel()
                withTimeout(5000) { closing.await() }
                assertFalse(finding.isCompleted); assertEquals(listOf(first), core.opened)
                finish.complete(Unit); withTimeout(5000) { finding.join() }
                assertTrue(finding.isCancelled); assertTrue(project.closeFinished); assertEquals(1, project.closes)
            } finally { finish.complete(Unit); finding.cancelAndJoin() }
        }
    }

    @Test fun failedClosePropagatesForBothMatchingAndMismatchingCandidates() = runBlocking {
        for (matches in listOf(true, false)) Fixture().use { fixture ->
            val first = fixture.project(1); fixture.project(2)
            val error = IllegalStateException("Fixture uncertain close")
            val project = Project({ if (matches) info() else info().copy(hostSeq = 999uL) }, { throw error })
            val core = Core { project }
            failed(error) { findRetainedMcpCaptureProject(fixture.root, expected, core, McpCaptureProjectRetirement()) }
            assertEquals(listOf(first), core.opened)
            assertEquals(1, project.closes); assertFalse(project.closeFinished)
        }
    }

    @Test fun failedRecoveryCloseKeepsExactOwnerAndFencesEveryLaterOpenUntilRetired() = runBlocking {
        Fixture().use { fixture ->
            val first = fixture.project(1); val second = fixture.project(2)
            val failure = IllegalStateException("First native close failed before destroy")
            var mayClose = false
            val uncertain = Project({ info().copy(hostSeq = 999uL) }, { if (!mayClose) throw failure })
            val matched = Project({ info() })
            val core = Core { path -> if (path == first) uncertain else matched }
            val retirement = McpCaptureProjectRetirement()
            failed(failure) { findRetainedMcpCaptureProject(fixture.root, expected, core, retirement) }
            assertEquals(listOf(first), core.opened)
            failed(failure) { findRetainedMcpCaptureProject(fixture.root, expected, core, retirement) }
            assertEquals(listOf(first), core.opened)
            // The same owner is retried before scanning or opening a second
            // candidate. A shutdown can settle it even after recovery failed.
            mayClose = true
            retirement.settle()
            assertEquals(3, uncertain.closes); assertTrue(uncertain.closeFinished)
            assertEquals(second, findRetainedMcpCaptureProject(fixture.root, expected, core, retirement))
            assertEquals(listOf(first, first, second), core.opened)
            assertTrue(matched.closeFinished)
        }
    }

    @Test fun actualSourceBorrowCallerRetainsFailedCloseBeforeASecondProjectOpen() = runBlocking {
        Fixture().use { fixture ->
            val first = fixture.project(1); val second = fixture.project(2)
            val failure = IllegalStateException("Uncertain source close")
            var mayClose = false
            val uncertain = Project({ info() }, { if (!mayClose) throw failure })
            val matched = Project({ info() })
            val core = Core { path -> if (path == first) uncertain else matched }
            val retirement = McpCaptureProjectRetirement()
            failed(failure) { retirement.borrow(core, listOf(first to expected)) { assertSame(uncertain, it.single()) } }
            assertEquals(listOf(first), core.opened)
            failed(failure) { retirement.borrow(core, listOf(second to expected)) { fail("Second borrow escaped failed close") } }
            assertEquals(listOf(first), core.opened)
            mayClose = true
            retirement.borrow(core, listOf(second to expected)) { assertSame(matched, it.single()) }
            assertEquals(listOf(first, second), core.opened)
            assertEquals(3, uncertain.closes); assertTrue(uncertain.closeFinished && matched.closeFinished)
        }
    }

    @Test fun twoSourceBorrowSettlesBothAndRetainsOnlyTheUncertainOwner() = runBlocking {
        Fixture().use { fixture ->
            val first = fixture.project(1); val second = fixture.project(2)
            val failure = IllegalStateException("After source close failed")
            var mayClose = false
            val before = Project({ info() }); val after = Project({ info() }, { if (!mayClose) throw failure })
            val core = Core { path -> if (path == first) before else after }
            val retirement = McpCaptureProjectRetirement()
            failed(failure) { retirement.borrow(core, listOf(first to expected, second to expected)) {
                assertEquals(listOf(before, after), it)
            } }
            assertEquals(listOf(first, second), core.opened)
            assertEquals(1, before.closes); assertTrue(before.closeFinished)
            failed(failure) { retirement.settle() }
            assertEquals(1, before.closes)
            mayClose = true
            retirement.settle()
            assertEquals(3, after.closes); assertTrue(after.closeFinished)
        }
    }
}
