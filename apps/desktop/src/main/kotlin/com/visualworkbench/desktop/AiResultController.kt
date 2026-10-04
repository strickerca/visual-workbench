package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

internal data class AiResultImage<T>(val objectId: String, val resultId: String, val compositeAssetId: String,
    val width: UInt, val height: UInt, val tiles: List<AiDisplayTile<T>>)
internal data class AiResultFrame<T>(val attachment: AiAttachment, val camera: Camera,
    val images: Map<String, AiResultImage<T>>, val poses: Map<String, Transform>) {
    fun matches(owner: AiAttachment?, view: Camera, preview: Map<String, Transform>): Boolean = attachment.sameProject(owner) &&
        attachment.binding == owner?.binding && camera == view && poses.all { (id, pose) ->
            (preview[id] ?: owner?.document?.render?.items?.firstOrNull { it.objectId == id }?.transform) == pose
        }
}
internal data class AiResultState<T>(val frame: AiResultFrame<T>? = null, val loading: Boolean = false,
    val message: String? = null)
internal data class AiResultPlan(val item: RenderItem, val shape: Shape.Result, val tiles: AiTilePlan)

/** Volatile preview messages cannot introduce an unpublished Result/asset or
 * add an annotation to a materialized Result's dedicated layer. Such pixels
 * become readable only after the corresponding canonical revision arrives. */
internal fun aiAdmitResultPreview(document: DocumentSnapshot, item: RenderItem): Boolean {
    val prior = document.render.items.firstOrNull { it.objectId == item.objectId }
    val resultLayer = document.render.items.any { it.layerId == item.layerId && it.shape is Shape.Result }
    if (resultLayer && prior?.shape !is Shape.Result) return false
    if (item.shape !is Shape.Result && prior?.shape !is Shape.Result) return true
    return prior != null && prior.shape is Shape.Result && item.shape == prior.shape &&
        item.layerId == prior.layerId && item.layerBlend == prior.layerBlend && item.layerOpacity == prior.layerOpacity
}

/** Affine inversion is used only for conservative tile admission. Camera
 * physical/D mapping comes from WorkbenchCore; painting uses its same transform.
 * Native remains responsible for Result binding, pixel conversion and proof. */
internal fun aiResultPlans(document: DocumentSnapshot, documentCorners: List<Point>, preview: Map<String, Transform> = emptyMap()): List<AiResultPlan> {
    val results = document.render.items.asSequence().filter { it.shape is Shape.Result }.take(9).toList()
    if (results.size > 8) throw AiDisplayRefusal(AiDisplayRefusal.Reason.ZoomIn)
    var remaining = AI_DISPLAY_BYTES
    return results.map { item ->
        val shape = item.shape as Shape.Result
        if (shape.resultId.isEmpty() || shape.resultId.length > 36 || shape.assetId.length != 64 ||
            item.layerBlend != "normal" || !item.layerOpacity.isFinite() || item.layerOpacity !in 0.0..1.0 ||
            document.render.items.count { it.layerId == item.layerId } != 1)
            throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
        val t = preview[item.objectId] ?: item.transform
        if (listOf(t.a, t.b, t.c, t.d, t.e, t.f).any { !it.isFinite() })
            throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
        val determinant = t.a * t.d - t.b * t.c
        if (!determinant.isFinite() || determinant == 0.0) throw AiDisplayRefusal(AiDisplayRefusal.Reason.Invalid)
        val local = documentCorners.map { p ->
            Point((t.d * (p.x - t.e) - t.c * (p.y - t.f)) / determinant,
                (-t.b * (p.x - t.e) + t.a * (p.y - t.f)) / determinant)
        }
        val plan = aiTilePlan(document.width, document.height, local, maxOf(4, remaining))
        if (plan.rgbaBytes > remaining) throw AiDisplayRefusal(AiDisplayRefusal.Reason.ZoomIn)
        remaining -= plan.rgbaBytes
        AiResultPlan(item.copy(transform = t), shape, plan)
    }
}

/** Provider-independent, lazy project reader. Opening an ordinary Result never
 * touches key settings, a provider service or budget-history initialization.
 * One active read and one latest viewport; detach joins before project close. */
internal class AiResultController<T>(
    private val scope: CoroutineScope,
    private val core: WorkbenchCore,
    private val attachment: () -> AiAttachment?,
    private val camera: () -> Camera,
    private val preview: () -> Map<String, Transform> = { emptyMap() },
    private val image: suspend (AiPixels) -> T,
    private val abandonImage: (T) -> Unit = {},
    private val results: (WorkbenchProject) -> WorkbenchAiResults = { it.aiResults() },
) {
    private val mutable = MutableStateFlow(AiResultState<T>())
    val state: StateFlow<AiResultState<T>> = mutable.asStateFlow()
    private val reader = AiLatestTask(scope)
    private var ticket = 0L
    private var closed = false
    private var retiring = false
    private var retirements = 0
    private var requested: AiResultFrame<T>? = null

    fun request() {
        if (closed || retiring || !scope.isActive) return
        val owner = attachment() ?: run { invalidate(); return }
        val view = camera()
        val proposed = preview()
        val poses = owner.document.render.items.asSequence().filter { it.shape is Shape.Result }.take(9)
            .associate { it.objectId to (proposed[it.objectId] ?: it.transform) }
        if (mutable.value.frame?.matches(owner, view, poses) == true) return
        // Publishing a reader's Loading state can cause the editor to publish
        // another scene. Identical in-flight requests must not cancel each
        // other indefinitely. A failed request also waits for explicit Retry
        // or a changed document/view instead of creating a hot retry loop.
        if (requested?.matches(owner, view, poses) == true) return
        requested = AiResultFrame(owner, view, emptyMap(), poses)
        val entered = ++ticket
        mutable.value = AiResultState(loading = true)
        reader.offer {
            val acquired = ArrayList<T>()
            var published = false
            try {
                val corners = core.mapPoints(view, true, listOf(Point(0.0, 0.0), Point(view.viewportWidth, 0.0),
                    Point(view.viewportWidth, view.viewportHeight), Point(0.0, view.viewportHeight)))
                val plan = aiResultPlans(owner.document, corners, poses)
                val frames = linkedMapOf<String, AiResultImage<T>>()
                // Resolve this capability only if the document actually has a
                // Result. Existing fake/older projects remain usable unchanged.
                if (plan.isNotEmpty()) {
                    val api = results(owner.project)
                    val context = api.context(owner.binding)
                    if (context.binding != owner.binding || context.width != owner.document.width || context.height != owner.document.height)
                        throw AiFailure(AiFailureKind.Proof)
                    for (item in plan) {
                        val tiles = ArrayList<AiDisplayTile<T>>(item.tiles.regions.size)
                        for (region in item.tiles.regions) {
                            currentCoroutineContext().ensureActive()
                            val value = api.resultPixels(owner.binding, item.shape.resultId, region)
                            if (value.binding != owner.binding || value.resultId != item.shape.resultId ||
                                value.compositeAssetId != item.shape.assetId || value.sourceAssetId != context.sourceAssetId ||
                                value.region != region || value.canvasWidth != context.width || value.canvasHeight != context.height)
                                throw AiFailure(AiFailureKind.Proof)
                            aiCheckPixels(region, value.canvasWidth, value.canvasHeight, value.rgbaSrgb)
                            val bitmap = image(AiPixels(value.resultId, region, value.canvasWidth, value.canvasHeight, value.rgbaSrgb))
                            acquired += bitmap
                            tiles += AiDisplayTile(region, bitmap)
                        }
                        frames[item.item.objectId] = AiResultImage(item.item.objectId, item.shape.resultId,
                            item.shape.assetId, context.width, context.height, tiles)
                    }
                }
                currentCoroutineContext().ensureActive()
                val frame = AiResultFrame(owner, view, frames, plan.associate { it.item.objectId to it.item.transform })
                if (entered != ticket || closed || retiring || !frame.matches(attachment(), camera(), preview())) return@offer
                mutable.value = AiResultState(frame = frame)
                published = true
            } catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) {
                if (entered == ticket && !closed && !retiring) mutable.value = AiResultState(message = when (error) {
                    is AiDisplayRefusal -> if (error.reason == AiDisplayRefusal.Reason.ZoomIn)
                        "Saved Result display exceeds 64 MiB. Zoom in; original pixels and exports are preserved."
                        else "This Result layer cannot be displayed with its current geometry. Saved bytes are preserved."
                    is AiFailure -> aiFailureText(error.kind)
                    else -> "Saved Result display is unavailable. Reopen or retry; saved bytes are preserved."
                })
            } finally {
                if (!published) acquired.forEach(abandonImage)
            }
        }
    }
    fun retry() { requested = null; request() }
    fun invalidate() { ticket++; requested = null; reader.cancelPending(); mutable.value = AiResultState() }
    suspend fun detach() {
        retirements++; retiring = true; invalidate()
        try { withContext(NonCancellable) { reader.pause() } }
        finally { retirements--; retiring = retirements != 0 }
    }
    suspend fun close() { closed = true; invalidate(); withContext(NonCancellable) { reader.close() } }
}
