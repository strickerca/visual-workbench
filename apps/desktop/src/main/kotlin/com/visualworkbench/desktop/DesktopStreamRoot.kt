package com.visualworkbench.desktop

import com.visualworkbench.desktop.stream.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.CoroutineScope

/** Narrow native acquisition boundary, injectable for real root lifecycle
 * tests without a JNI link or a platform window. */
internal interface DesktopStreamReceiverApi {
    fun connectionEpoch(link:ProjectLink):ULong=markerFocus(link).signal().connectionEpoch
    suspend fun open(link:ProjectLink):WorkbenchCaptureStream=link.openCaptureStream()
    fun annotation(link:ProjectLink,target:WorkbenchProject,documentId:String,stream:WorkbenchCaptureStream,
        newId:()->String,metadata:suspend(ProjectInfo)->WorkflowMetadata,current:()->Boolean):CaptureAnnotationSession =
        link.captureAnnotationSession(target,documentId,stream,newId,metadata,current)
    fun renderer(scope:CoroutineScope,core:WorkbenchCore,stream:WorkbenchCaptureStream,current:()->Boolean):DesktopStreamRenderPort =
        DesktopStreamRenderAdapter(DesktopStreamView(scope,core,stream,current))
}
internal object NativeDesktopStreamReceiverApi:DesktopStreamReceiverApi

/** A stream Save can contain at most 4,096 operations. Reserve its complete
 * range once before native plan preparation, including unknown-outcome retries.
 * The floor survives stream close/reopen for this controller. Ordinary edits
 * continue using native nextLamport after every stream plan has retired. */
internal class DesktopStreamSaveCounters {
    private var next = 1uL
    fun reserve(minimum: ULong): ULong {
        val first = maxOf(next, minimum)
        require(first > 0uL && first <= ULong.MAX_VALUE - 4096uL)
        next = first + 4096uL
        return first
    }
}

internal data class DesktopStreamRootState(
    val session: DesktopStreamSession? = null,
    val opening: Boolean = false,
    val cleanupPending: Boolean = false,
    val issue: String? = null,
) {
    val blocksParent: Boolean get() = opening || cleanupPending || session != null
}

/** Native unload must refuse a still-owned receiver, including failed or late
 * acquisition. Membership is removed only after joined retirement succeeds. */
private val desktopStreamOwners = mutableSetOf<Any>()
internal fun retainDesktopStreamRoot(owner: Any) = synchronized(desktopStreamOwners) { desktopStreamOwners.add(owner); Unit }
internal fun retireDesktopStreamRoot(owner: Any) = synchronized(desktopStreamOwners) { desktopStreamOwners.remove(owner); Unit }
internal fun assertDesktopStreamsRetired() = synchronized(desktopStreamOwners) {
    check(desktopStreamOwners.isEmpty()) { "Desktop capture stream retirement is incomplete" }
}

internal class DesktopStreamRenderAdapter(override val view: DesktopStreamView):DesktopStreamRenderPort {
    override suspend fun stopForContact(point:Point)=view.stopForContact(point)
    override suspend fun adoptLossless(annotation:CaptureAnnotationSession)=view.adoptLossless(annotation)
    override suspend fun close()=view.close()
}

internal class DesktopStreamEditorAdapter(override val adapter:StreamDraftEditorAdapter):DesktopStreamEditorPort {
    override val ready:Boolean get()=adapter.ready
    override suspend fun sealEdits()=adapter.sealEdits()
    override suspend fun close()=adapter.close()
}
