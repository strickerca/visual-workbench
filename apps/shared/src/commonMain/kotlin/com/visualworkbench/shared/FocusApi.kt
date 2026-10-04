package com.visualworkbench.shared

import kotlinx.coroutines.flow.Flow

public data class FocusSignal(public val sequence: ULong, public val connectionEpoch: ULong, public val available: Boolean)
public data class PeerMarkerFocus(
    public val binding: WorkflowBinding, public val markerId: String, public val instructionId: String,
    public val sequence: ULong, public val connectionEpoch: ULong,
)
/** Borrows ProjectLink; closing the link invalidates this capability. No marker
 * text or camera coordinates cross this channel. Query again after publishing
 * a new exact visible revision: a hint can arrive before its accepted edit. */
public interface WorkbenchMarkerFocus {
    public val signals: Flow<FocusSignal>
    public fun signal(): FocusSignal
    /** Owner-authorized local selection only. Null explicitly clears selection.
     * Latest input coalesces and the carrier sends at most 20 updates/second.
     * Never echo a received selection through this method. */
    public suspend fun send(binding: WorkflowBinding, markerId: String?)
    /** Returns a live numbered marker and its canonical linked instruction only
     * when the complete project/document/host-sequence/hash binding matches. */
    public suspend fun peer(binding: WorkflowBinding): PeerMarkerFocus?
}
public expect fun markerFocus(link: ProjectLink): WorkbenchMarkerFocus
