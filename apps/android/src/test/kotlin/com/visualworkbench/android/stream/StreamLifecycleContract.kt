package com.visualworkbench.android.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.flow

/** Also callable by the bounded offline compiler probe without Android/JNI. */
internal fun runStreamLifecycleContract(): Int = runBlocking {
    withTimeout(10_000) {
        var cases = 0
        run {
            val f = StreamFixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            f.presentHook = { entered.complete(Unit); release.await(); true }
            val draw = async { f.owner.drawLive() }; entered.await()
            val freeze = async { f.owner.freeze() }; yield()
            check(f.receiver.contacts == 0 && f.receiver.tickets.single().choices.isEmpty())
            release.complete(Unit); draw.await(); freeze.await()
            check(f.order.indexOf("ack:true:1") < f.order.indexOf("contact"))
            f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive()
            f.receiver.nextFrame = f.frame.copy(frameId = 2uL)
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            f.factory.copyHook = { entered.complete(Unit); release.await() }
            val draw = async { f.owner.drawLive() }; entered.await()
            val freeze = async { f.owner.freeze() }; yield()
            release.complete(Unit); draw.await(); freeze.await()
            check(f.receiver.tickets.last().choices == listOf(false))
            check(f.owner.displayedFrame == f.frame)
            f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); f.presentHook = { error("unknown post outcome") }
            check(runCatching { f.owner.drawLive() }.isFailure)
            check(f.owner.sealed && f.receiver.tickets.single().choices.isEmpty())
            check(runCatching { f.owner.freeze() }.isFailure)
            f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); val entered = CompletableDeferred<Unit>(); val held = CompletableDeferred<Unit>()
            f.factory.copyHook = { entered.complete(Unit); held.await() }
            val draw = launch { f.owner.drawLive() }; entered.await(); draw.cancelAndJoin()
            check(f.receiver.tickets.single().choices == listOf(false) && f.owner.displayedFrame == null && !f.owner.sealed)
            f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive(); f.owner.freeze()
            var camera = 7; var provisional = "wet stroke"
            f.presentHook = { check(camera == 7 && provisional == "wet stroke"); true }
            f.receiver.originalReady = true
            f.owner.refreshOriginal()
            check(f.owner.losslessDisplayed && f.owner.displayedFrame == f.frame)
            check(f.factory.images.count { it.closed } == 1)
            camera = 9; provisional = "moved stroke" // The platform keeps the current scene slot.
            f.presentHook = { check(camera == 9 && provisional == "moved stroke"); true }
            f.owner.drawFrozen(); f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive(); f.owner.freeze()
            f.receiver.originalReady = true; f.receiver.originalFrame = f.frame.copy(sourceGeneration = 2uL)
            check(runCatching { f.owner.refreshOriginal() }.isFailure)
            check(!f.owner.losslessDisplayed && f.receiver.backgrounds.all { it.closes == 1 })
            f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive(); f.owner.freeze()
            f.receiver.stage = CaptureStreamStage.SourceClosed
            val count = f.posts
            f.owner.drawFrozen()
            check(f.posts == count + 1 && f.owner.displayedFrame == f.frame)
            f.owner.close(); cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive()
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            f.retireSurfaceHook = { entered.complete(Unit); release.await() }
            val close = async { f.owner.close() }; entered.await()
            check(f.factory.images.none { it.closed } && f.receiver.closes == 0)
            release.complete(Unit); close.await()
            check(f.factory.images.all { it.closed } && f.receiver.closes == 1)
            cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive(); val image = f.factory.images.single(); image.failClose = true
            check(runCatching { f.owner.close() }.isFailure)
            check(!image.closed && f.receiver.closes == 0)
            f.owner.close(); check(image.closed && image.closeAttempts == 2 && f.receiver.closes == 1)
            cases++
        }
        run {
            val f = StreamFixture(); f.owner.drawLive(); f.owner.freeze()
            f.retireChildHook = { error("child renderer still retained") }
            check(runCatching { f.owner.resume() }.isFailure); check(f.receiver.resumes == 0)
            f.retireChildHook = {}; f.owner.resume(); check(f.receiver.resumes == 1 && f.owner.displayedFrame == null)
            f.owner.close(); cases++
        }
        cases
    }
}

internal fun runStreamClosedLifecycleContract(): Int = runBlocking {
    withTimeout(10_000) {
        run {
            val f = StreamFixture(); f.owner.drawLive(); f.owner.freeze()
            val initial = f.receiver.signal()
            observeStreamSignals(initial, flow { throw SessionFailure(SessionFailureKind.Closed) }) {
                if (it.stage == CaptureStreamStage.Closed) f.owner.retireReceiver()
            }
            f.receiver.signalClosed = true; val reads = f.receiver.signalReads
            val posts = f.posts
            f.owner.drawLive(); f.owner.refreshOriginal(); f.owner.drawFrozen()
            check(runCatching { f.owner.activate() }.isFailure && runCatching { f.owner.freeze() }.isFailure)
            check(runCatching { f.owner.resume() }.isFailure && runCatching { f.owner.save() }.isFailure)
            check(f.owner.receiverClosed && f.receiver.signalReads == reads && f.posts == posts + 1)
            check(f.owner.displayedFrame == f.frame && f.factory.images.none { it.closed })
            f.owner.close(); check(f.factory.images.all { it.closed } && f.receiver.closes == 1)
        }
        run {
            val f = StreamFixture(); f.owner.drawLive()
            val initial = f.receiver.signal(); f.receiver.nextFrame = f.frame.copy(frameId = 2uL)
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            f.factory.copyHook = { entered.complete(Unit); release.await() }
            val draw = async { runCatching { f.owner.drawLive() } }; entered.await()
            observeStreamSignals(initial, flow { throw SessionFailure(SessionFailureKind.Closed) }) {
                if (it.stage == CaptureStreamStage.Closed) f.owner.retireReceiver()
            }
            f.receiver.signalClosed = true; release.complete(Unit)
            val failure = draw.await().exceptionOrNull()
            check(failure is SessionFailure && failure.kind == SessionFailureKind.Closed)
            check(f.receiver.tickets.last().choices == listOf(false) && f.owner.displayedFrame == f.frame)
            check(f.factory.images.first().closed.not() && f.factory.images.last().closed)
            f.owner.close(); check(f.factory.images.all { it.closed })
        }
        2
    }
}

private class StreamFixture {
    val frame = CaptureStreamFrame("capture", 1uL, 1uL, 8u, 8u, 0, 0, 1.0, 1u, 20, 1000, "windows")
    val order = mutableListOf<String>()
    val receiver = Receiver(frame, order)
    val factory = Images()
    var presentHook: suspend () -> Boolean = { true }
    var retireChildHook: suspend () -> Unit = {}
    var retireSurfaceHook: suspend () -> Unit = {}
    var attached = true
    var posts = 0
    val draft = Draft(frame)
    val annotation = CaptureAnnotationSession(Project("paired"), "target", Link(), receiver,
        { "draft" }, { WorkflowMetadata("save", "device", 1uL, 1000) }, { attached }, { _, _, _, _ -> draft })
    val owner = StreamCanvasLifecycle(annotation, factory, { _, _, _ -> posts++; presentHook() },
        { retireChildHook() }, { retireSurfaceHook() }, { attached })
}

private class Image(override val frame: CaptureStreamFrame) : StreamOwnedPixels<String> {
    override val value = "pixels:${frame.frameId}"
    var closed = false; var closeAttempts = 0; var failClose = false
    override suspend fun close() {
        closeAttempts++
        if (failClose) { failClose = false; error("charge retirement pending") }
        closed = true
    }
}
private class Images : StreamPixelFactory<String> {
    val images = mutableListOf<Image>()
    var copyHook: suspend () -> Unit = {}
    override suspend fun copy(source: CaptureStreamPixels): StreamOwnedPixels<String> {
        copyHook(); return Image(source.frame).also { images += it }
    }
    override suspend fun close() { }
}
private class Pixels(override val frame: CaptureStreamFrame, override val lossless: Boolean) : CaptureFrozenBackground {
    override val draftId = "draft"
    override val originalAssetId = if (lossless) "b".repeat(64) else null
    var closes = 0
    override suspend fun rgbaRows(firstRow: UInt, rowCount: UInt) = error("fake factory owns rows")
    override suspend fun close() { closes++ }
}
private class Ticket(override val frame: CaptureStreamFrame, private val order: MutableList<String>,
    private val acknowledge: (CaptureStreamFrame) -> Unit) : CaptureStreamDraw {
    val choices = mutableListOf<Boolean>()
    override suspend fun rgbaRows(firstRow: UInt, rowCount: UInt) = error("fake factory owns rows")
    override suspend fun presented(actuallyShown: Boolean) {
        choices += actuallyShown; order += "ack:$actuallyShown:${frame.frameId}"
        if (actuallyShown) acknowledge(frame)
    }
    override suspend fun close() { order += "ticket-close" }
}
private class Receiver(val frame: CaptureStreamFrame, private val order: MutableList<String>) : WorkbenchCaptureStream {
    var nextFrame = frame; var shown: CaptureStreamFrame? = null
    var stage = CaptureStreamStage.Live; var contacts = 0; var closes = 0; var resumes = 0
    var originalReady = false; var originalFrame = frame
    var signalClosed = false; var signalReads = 0
    val tickets = mutableListOf<Ticket>(); val backgrounds = mutableListOf<Pixels>()
    override val signals = emptyFlow<CaptureStreamSignal>()
    override fun signal(): CaptureStreamSignal {
        signalReads++
        if (signalClosed) throw SessionFailure(SessionFailureKind.Closed)
        return CaptureStreamSignal(1uL, 1uL, 1uL, nextFrame.frameId, stage, 8u, 8u, 256uL, null)
    }
    override suspend fun beginDraw() = Ticket(nextFrame, order) { shown = it }.also { tickets += it }
    override suspend fun frozenBackground(): CaptureFrozenBackground? =
        if (originalReady) Pixels(originalFrame, true).also { backgrounds += it } else null
    override suspend fun firstContact(draftId: String): CaptureFreezeReceipt {
        order += "contact"; contacts++; stage = CaptureStreamStage.Freezing
        val current = checkNotNull(shown)
        return CaptureFreezeReceipt(draftId, current.frameId, current.captureSessionId, current.sourceGeneration)
    }
    override suspend fun resumeAfterDraftDisposed(): String { resumes++; stage = CaptureStreamStage.Live; return "draft" }
    override suspend fun close() { closes++; stage = CaptureStreamStage.Closed }
}
private class Draft(frame: CaptureStreamFrame) : WorkbenchCaptureDraft {
    override val info = CaptureDraftInfo("draft", "document", frame, "a".repeat(64), 1uL)
    override val project = Project("draft")
    override suspend fun prepareSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        original: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave = error("unused")
    override suspend fun prepareDegradedSave(source: WorkflowBinding, target: WorkflowBinding, metadata: WorkflowMetadata,
        preview: CaptureFrozenBackground, memoryBudgetBytes: ULong): WorkbenchCaptureDraftSave = error("unused")
    override suspend fun close() { }
}
private class Project(private val id: String) : WorkbenchProject {
    override val changes = emptyFlow<ProjectChange>()
    override suspend fun info() = ProjectInfo(id, "project", "device", 1uL, false, false, 0uL, "a".repeat(64), listOf("document"))
    override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("unused")
    override suspend fun close() { }
}
private class Link : ProjectLink {
    override val endpoints = emptyList<SessionEndpoint>(); override val changes = emptyFlow<SessionStatus>()
    override fun status(): SessionStatus = error("unused")
    override fun sendViewport(documentId: String, corners: List<Point>) { }
    override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) { }
    override fun streamObject(preview: ObjectPreview) { }
    override fun streamNewObject(preview: NewObjectPreview) { }
    override suspend fun finishPreview(gestureId: String, cancel: Boolean) { }
    override suspend fun peerPreviews(documentId: String): PeerPreviews = error("unused")
    override suspend fun close() { }
}
