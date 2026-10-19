package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

/** A frame's content contract is checked with the existing owner and draw fences. */
class GiantComposedViewportBoundaryTest {
    private val binding = WorkflowBinding("project", "document", 7uL, "c".repeat(64))
    private val viewport = GiantViewport(0.0, 0.0, 256.0, 256.0, 1.0, 0.0, 0.0, 1.0)

    @Test fun composedFrameRequiresMatchingOwnerModeAndCommittedRevision() {
        val frame = GiantVisibleFrame<Int>(binding, 3L, 9uL, viewport,
            "a".repeat(64), "b".repeat(64), emptyList(), GiantTileContent.Composed)
        val ready = GiantViewStatus(generation = 9uL, ready = true)
        fun visible(candidate: GiantVisibleFrame<Int>? = frame, owned: GiantVisibleFrame<Int>? = frame,
            expected: WorkflowBinding = binding, mode: GiantTileContent = GiantTileContent.Composed,
            status: GiantViewStatus = ready) = giantFrameCurrent(candidate, owned, expected, 3L,
            viewport, status, admitted = true, current = true, content = mode)
        assertSame(frame, visible())
        assertNull(visible(mode = GiantTileContent.Source))
        assertNull(visible(candidate = frame.copy(content = GiantTileContent.Source)))
        assertNull(visible(owned = frame.copy()))
        assertNull(visible(expected = binding.copy(stateHash = "d".repeat(64))))
        assertNull(visible(expected = binding.copy(hostSeq = 8uL)))
        assertNull(visible(status = ready.copy(generation = 10uL)))
    }

    @Test fun oldSourceFrameKeepsTheSourceDefault() {
        val source = GiantVisibleFrame<Int>(binding, 3L, 1uL, viewport,
            "a".repeat(64), "b".repeat(64), emptyList())
        assertEquals(GiantTileContent.Source, source.content)
        assertSame(source, giantFrameCurrent(source, source, binding, 3L, viewport,
            GiantViewStatus(generation = 1uL, ready = true), admitted = true, current = true))
        assertNull(giantFrameCurrent(source, source, binding, 3L, viewport,
            GiantViewStatus(generation = 1uL, ready = true), admitted = true, current = true,
            content = GiantTileContent.Composed))
    }
}
