package com.visualworkbench.android.input

import android.view.MotionEvent
import org.junit.Assert.assertEquals
import org.junit.Test

class CanvasGestureCancellationTest {
    @Test fun normalContactsDoNotCancelAnIdleDraftButExplicitCancelDoes() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        repeat(3) { index ->
            router.accept(packet(MotionEvent.ACTION_DOWN, 100L + index * 10))
            router.accept(packet(MotionEvent.ACTION_UP, 101L + index * 10))
        }
        assertEquals(3, sink.finished); assertEquals(0, sink.canceled)
        router.cancel()
        assertEquals(1, sink.canceled)
    }
    @Test fun canceledPacketAlsoCancelsAnIdleDraftBetweenContacts() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        router.accept(packet(MotionEvent.ACTION_DOWN, 100))
        router.accept(packet(MotionEvent.ACTION_UP, 101))
        router.accept(packet(MotionEvent.ACTION_CANCEL, 102, true))
        assertEquals(1, sink.canceled)
    }
    @Test fun newDownDuringAnActiveContactCancelsTheUnfinishedContact() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        router.accept(packet(MotionEvent.ACTION_DOWN, 100))
        router.accept(packet(MotionEvent.ACTION_DOWN, 101))
        assertEquals(1, sink.canceled)
    }
    private fun packet(action: Int, time: Long, canceled: Boolean = false): PointerPacket {
        val sample = PenSample(1, PointerTool.Pen, 2.0, 3.0, time, .5f, .5f, null, null, 0)
        return PointerPacket(action, 1, canceled, time, listOf(PointerFrame(time, listOf(sample))),
            StylusCapabilities(true, false, false, false, false, false))
    }
    private class Sink : CanvasGestureSink {
        var canceled = 0; var finished = 0
        override fun beginTool(sample: PenSample, capabilities: StylusCapabilities) = Unit
        override fun extendTool(samples: List<PenSample>) = Unit
        override fun finishTool(samples: List<PenSample>) { finished++ }
        override fun cancelTool() { canceled++ }
        override fun pan(dx: Double, dy: Double) = Unit
        override fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double) = Unit
        override fun undo() = Unit
        override fun redo() = Unit
        override fun contextMenu(x: Double, y: Double) = Unit
        override fun hover(sample: PenSample?, capabilities: StylusCapabilities) = Unit
    }
}
