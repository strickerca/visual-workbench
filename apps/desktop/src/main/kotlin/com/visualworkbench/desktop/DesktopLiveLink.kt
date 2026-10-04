package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*

data class PeerFrame(val binding: RenderBinding, val linkEpoch: Long, val previews: PeerPreviews)
internal class LiveAttachment(val epoch: Long, val link: ProjectLink) {
    var status = link.status(); private set
    var previewEpoch = 0L; private set
    var lastFrame: PeerFrame? = null
    val acceptsPreviews: Boolean get() = status.status in setOf(SyncStatus.Syncing, SyncStatus.Synced)

    fun observe(value: SessionStatus) {
        if (value.status != status.status || value.carrier != status.carrier) {
            check(previewEpoch < Long.MAX_VALUE)
            previewEpoch++
            lastFrame = null
        }
        status = value
    }
}

/** UI ownership only; packet ordering, authentication, replay and reconnect are
 * native responsibilities. Every callback is guarded by attachment identity. */
internal class DesktopLiveLink(
    private val scope: CoroutineScope,
    private val binding: () -> RenderBinding?,
    private val status: (SessionStatus?) -> Unit,
    private val previews: (PeerFrame?) -> Unit,
    private val warning: (String) -> Unit,
    private val beforeClose: suspend () -> Unit = {},
) {
    private var nextEpoch = 0L
    private var current: LiveAttachment? = null
    private var statusJob: Job? = null
    private var previewJob: Job? = null
    private val finishes = linkedSetOf<Job>()
    fun attachment(): LiveAttachment? = current

    suspend fun install(link: ProjectLink, checkTether: suspend () -> Unit) {
        close()
        check(nextEpoch < Long.MAX_VALUE)
        val attached = LiveAttachment(++nextEpoch, link)
        current = attached
        status(attached.status)
        statusJob = scope.launch {
            var last = 0uL
            var connectedTether = false
            try {
                link.changes.collect { value ->
                    if (current !== attached || value.sequence <= last) return@collect
                    last = value.sequence
                    attached.observe(value)
                    if (!attached.acceptsPreviews) {
                        attached.lastFrame = null
                        previews(null)
                    }
                    status(value)
                    val tether = value.carrier == SessionCarrier.QuicTether &&
                        value.status in setOf(SyncStatus.Syncing, SyncStatus.Synced)
                    if (tether && !connectedTether) {
                        try { checkTether() }
                        catch (cancel: CancellationException) { throw cancel }
                        catch (_: Exception) { warning("Tether route safety could not be checked. Review the selected interface before continuing.") }
                    }
                    connectedTether = tether
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (current === attached) warning(sessionMessage(error)) }
        }
        previewJob = scope.launch {
            try {
                while (currentCoroutineContext().isActive && current === attached) {
                    val requested = binding()
                    val requestedEpoch = attached.previewEpoch
                    if (requested != null && attached.acceptsPreviews) {
                        val value = try { link.peerPreviews(requested.documentId) }
                        catch (error: SessionFailure) {
                            if (error.kind !in volatileFailures) throw error
                            attached.lastFrame = null
                            previews(null); delay(16); continue
                        }
                        val last = attached.lastFrame
                        if (current === attached && attached.acceptsPreviews && requestedEpoch == attached.previewEpoch && requested == binding() &&
                            (last?.binding != requested || value.sequence > (last?.previews?.sequence ?: 0uL) || last == null)) {
                            val frame = PeerFrame(requested, attached.epoch, value)
                            attached.lastFrame = frame
                            previews(frame)
                        }
                    }
                    // One in-flight query, no accumulating timer callbacks.
                    delay(16)
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { if (current === attached) { previews(null); warning(sessionMessage(error)) } }
        }
    }

    fun sendViewport(document: String, corners: List<Point>) {
        try { current?.link?.sendViewport(document, corners) }
        catch (error: SessionFailure) { if (error.kind !in volatileFailures) warning(sessionMessage(error)) }
    }

    suspend fun finish(attached: LiveAttachment?, id: String?, cancel: Boolean) {
        if (attached == null || id == null || current !== attached) return
        try { attached.link.finishPreview(id, cancel) }
        catch (error: SessionFailure) { if (error.kind !in volatileFailures) warning(sessionMessage(error)) }
    }

    fun cancelLater(attached: LiveAttachment?, id: String?) {
        if (attached == null || id == null || current !== attached) return
        // At most a drag, slider and stroke can request closure. Stop admission
        // on pathological repeated cancellation rather than accumulating work.
        if (finishes.size >= 8) {
            warning("Preview cleanup is busy. The link is closing; saved edits remain on this device.")
            scope.launch(start = CoroutineStart.UNDISPATCHED) { close() }
            return
        }
        lateinit var job: Job
        job = scope.launch(start = CoroutineStart.LAZY) {
            try { finish(attached, id, true) } finally { finishes.remove(job) }
        }
        finishes += job
        job.start()
    }

    suspend fun close() = withContext(NonCancellable) {
        val prior = current
        current = null
        try { beforeClose() } finally {
        previews(null)
        status(null)
        statusJob?.cancelAndJoin(); statusJob = null
        previewJob?.cancelAndJoin(); previewJob = null
        finishes.toList().forEach { it.cancelAndJoin() }
        finishes.clear()
        prior?.link?.close()
        }
    }
}

private val volatileFailures = setOf(SessionFailureKind.Backpressure, SessionFailureKind.Closed, SessionFailureKind.Transport)

/** All intermediate transforms/styles remain UI-only until the one final edit.
 * One native generation binds one target, so multi-selection stays commit-only. */
internal interface GesturePreview {
    val id: String
    val attachment: LiveAttachment
}
internal class ObjectPreviewLease(
    override val id: String,
    override val attachment: LiveAttachment,
    private val document: String,
    private val objectId: String,
    private val kind: PreviewKind,
    private val nanoTime: () -> Long = System::nanoTime,
) : GesturePreview {
    private var sequence = 0u
    private var lastSent: Long? = null
    private var stopped = false
    fun update(transform: Transform, style: ObjectStyle) {
        if (stopped || sequence == UInt.MAX_VALUE) return
        val now = nanoTime()
        if (lastSent?.let { now - it < 8_333_334L } == true) return
        try {
            attachment.link.streamObject(ObjectPreview(id, document, objectId, ++sequence, kind, transform, style))
            lastSent = now
        } catch (_: SessionFailure) {
            // A suffix is not restarted on a new transport epoch. The final
            // durable edit still proceeds; the native generation expires.
            stopped = true
        }
    }
}

internal class NewObjectPreviewLease(
    override val id: String,
    override val attachment: LiveAttachment,
    val objectId: String,
    val document: String,
    val layer: String,
    val createdAt: Long,
    val shape: Shape,
    val style: ObjectStyle,
    private val nanoTime: () -> Long = System::nanoTime,
) : GesturePreview {
    private var sequence = 0u
    private var lastSent: Long? = null
    private var stopped = false
    fun update(geometry: Shape) {
        if (stopped || sequence == UInt.MAX_VALUE) return
        val now = nanoTime()
        if (lastSent?.let { now - it < 8_333_334L } == true) return
        try {
            require(geometry.javaClass == shape.javaClass)
            attachment.link.streamNewObject(NewObjectPreview(id, document, objectId, layer, ++sequence,
                createdAt, geometry, Transform(), style))
            lastSent = now
        } catch (_: SessionFailure) { stopped = true }
    }
}

internal fun draftTemplate(tool: Tool): Shape? = when (tool) {
    Tool.Rectangle -> Shape.Rectangle(Rect(0.0, 0.0, 1.0, 1.0))
    Tool.Ellipse -> Shape.Ellipse(Rect(0.0, 0.0, 1.0, 1.0))
    Tool.Line -> Shape.Line(listOf(Point(0.0, 0.0), Point(1.0, 0.0)))
    Tool.Arrow -> Shape.Arrow(listOf(Point(0.0, 0.0), Point(1.0, 0.0)))
    else -> null
}

internal fun draftShape(tool: Tool, start: Point, end: Point): Shape? {
    val dx = end.x - start.x; val dy = end.y - start.y
    return when (tool) {
        Tool.Rectangle, Tool.Ellipse -> if (kotlin.math.abs(dx) > .001 && kotlin.math.abs(dy) > .001) {
            val rect = Rect(minOf(start.x, end.x), minOf(start.y, end.y), kotlin.math.abs(dx), kotlin.math.abs(dy))
            if (tool == Tool.Rectangle) Shape.Rectangle(rect) else Shape.Ellipse(rect)
        } else null
        Tool.Line -> if (kotlin.math.hypot(dx, dy) > .001) Shape.Line(listOf(start, end)) else null
        Tool.Arrow -> if (kotlin.math.hypot(dx, dy) > .001) Shape.Arrow(listOf(start, end)) else null
        else -> null
    }
}

/** The wire sends document-space corners. Reconstruct a camera only for the
 * explicit follow/match command, fitting its whole visible rectangle locally. */
internal fun peerCamera(value: PeerViewport, local: Camera): Camera? {
    val p = value.corners
    if (p.size != 4 || p.any { !it.x.isFinite() || !it.y.isFinite() }) return null
    val width = kotlin.math.hypot(p[1].x - p[0].x, p[1].y - p[0].y)
    val height = kotlin.math.hypot(p[3].x - p[0].x, p[3].y - p[0].y)
    if (width <= 1e-9 || height <= 1e-9) return null
    return Camera(Point(p.sumOf { it.x } / 4, p.sumOf { it.y } / 4),
        minOf(local.viewportWidth / width, local.viewportHeight / height).coerceIn(.001, 256.0),
        -kotlin.math.atan2(p[1].y - p[0].y, p[1].x - p[0].x), local.viewportWidth, local.viewportHeight)
}
