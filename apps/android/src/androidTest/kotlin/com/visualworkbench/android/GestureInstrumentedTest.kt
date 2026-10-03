package com.visualworkbench.android

import android.view.InputDevice
import android.view.MotionEvent
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.android.editor.PressureCurve
import com.visualworkbench.android.input.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Synthetic software tests; no S Pen latency, pressure resolution or owner trace claims. */
@RunWith(AndroidJUnit4::class)
class GestureInstrumentedTest {
    private val capabilities = StylusCapabilities(true, false, false, false, false, false)
    private fun finger(id: Int, x: Double = 10.0, y: Double = 10.0, time: Long = 100) =
        PenSample(id, PointerTool.Finger, x, y, time, 1f, 1f, null, null, 0)
    private fun packet(action: Int, actionPointer: Int, vararg points: PenSample, canceled: Boolean = false) =
        PointerPacket(action, actionPointer, canceled, 100, listOf(PointerFrame(points.maxOf { it.timeMs }, points.toList())), capabilities)
    private class Sink : CanvasGestureSink {
        var started = 0; var committed = 0; var canceled = 0; var pans = 0; var pinches = 0
        var undo = 0; var redo = 0; var context = 0; var samples = 0; var hover = 0
        override fun beginTool(sample: PenSample, capabilities: StylusCapabilities) { started++ }
        override fun extendTool(samples: List<PenSample>) { this.samples += samples.size }
        override fun finishTool(samples: List<PenSample>) { committed++ }
        override fun cancelTool() { canceled++ }
        override fun pan(dx: Double, dy: Double) { pans++ }
        override fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double) { pinches++ }
        override fun undo() { undo++ }
        override fun redo() { redo++ }
        override fun contextMenu(x: Double, y: Double) { context++ }
        override fun hover(sample: PenSample?, capabilities: StylusCapabilities) { hover++ }
    }
    @Test fun fingersNavigateAndPenNeverPansEvenWithPalmBesideIt() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        router.accept(packet(MotionEvent.ACTION_DOWN, 1, finger(1)))
        router.accept(packet(MotionEvent.ACTION_MOVE, 1, finger(1, 30.0, time = 110)))
        router.accept(packet(MotionEvent.ACTION_UP, 1, finger(1, 30.0, time = 120)))
        assertEquals(0, sink.started); assertEquals(1, sink.pans)
        val pen = finger(7, time = 200).copy(tool = PointerTool.Pen, rawPressure = .6f, pressure = .6f)
        router.accept(packet(MotionEvent.ACTION_DOWN, 7, pen))
        router.accept(packet(MotionEvent.ACTION_POINTER_DOWN, 8, pen.copy(timeMs = 201), finger(8, time = 201).copy(tool = PointerTool.Palm)))
        router.accept(packet(MotionEvent.ACTION_MOVE, 7, pen.copy(x = 40.0, timeMs = 202), finger(8, time = 202).copy(tool = PointerTool.Palm)))
        router.accept(packet(MotionEvent.ACTION_POINTER_UP, 8, pen.copy(timeMs = 203), finger(8, time = 203).copy(tool = PointerTool.Palm), canceled = true))
        router.accept(packet(MotionEvent.ACTION_UP, 7, pen.copy(x = 40.0, timeMs = 204)))
        assertEquals(1, sink.started); assertEquals(1, sink.committed); assertEquals(0, sink.canceled); assertEquals(1, sink.pans)
    }
    @Test fun canceledPenAndFingerTransitionCommitNothing() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        val pen = finger(1).copy(tool = PointerTool.Pen)
        router.accept(packet(MotionEvent.ACTION_DOWN, 1, pen))
        router.accept(packet(MotionEvent.ACTION_UP, 1, pen.copy(timeMs = 110), canceled = true))
        assertEquals(1, sink.canceled); assertEquals(0, sink.committed)
        router.drawWithFinger = true
        router.accept(packet(MotionEvent.ACTION_DOWN, 1, finger(1, time = 200)))
        router.accept(packet(MotionEvent.ACTION_POINTER_DOWN, 2, finger(1, time = 210), finger(2, 20.0, time = 210)))
        router.accept(packet(MotionEvent.ACTION_POINTER_UP, 2, finger(1, time = 220), finger(2, 20.0, time = 220), canceled = true))
        router.accept(packet(MotionEvent.ACTION_UP, 1, finger(1, time = 230)))
        assertEquals(2, sink.canceled); assertEquals(0, sink.committed); assertEquals(0, sink.undo)
    }
    @Test fun multiFingerTapsRouteUndoRedoAndPinchIsNotAnUndo() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        fun tap(count: Int) {
            for (n in 1..count) router.accept(packet(if (n == 1) MotionEvent.ACTION_DOWN else MotionEvent.ACTION_POINTER_DOWN,
                n, *(1..n).map { finger(it, it * 20.0, time = 100L + n) }.toTypedArray()))
            for (n in count downTo 1) router.accept(packet(if (n == 1) MotionEvent.ACTION_UP else MotionEvent.ACTION_POINTER_UP,
                n, *(1..n).map { finger(it, it * 20.0, time = 110L + count - n) }.toTypedArray()))
        }
        tap(2); tap(3); assertEquals(1, sink.undo); assertEquals(1, sink.redo)
        router.accept(packet(MotionEvent.ACTION_DOWN, 1, finger(1)))
        router.accept(packet(MotionEvent.ACTION_POINTER_DOWN, 2, finger(1), finger(2, 30.0)))
        router.accept(packet(MotionEvent.ACTION_MOVE, 2, finger(1, time = 120), finger(2, 80.0, time = 120)))
        router.accept(packet(MotionEvent.ACTION_POINTER_UP, 2, finger(1, time = 130), finger(2, 80.0, time = 130)))
        router.accept(packet(MotionEvent.ACTION_UP, 1, finger(1, time = 140)))
        assertEquals(1, sink.pinches); assertEquals(1, sink.undo)
    }
    @Test fun longPressAndHoverDoNotStartDrawing() {
        val sink = Sink(); val router = CanvasGestureRouter(sink, 4.0)
        router.accept(packet(MotionEvent.ACTION_DOWN, 1, finger(1)))
        router.longPress(601); router.longPress(700)
        assertEquals(1, sink.context); assertEquals(0, sink.started)
        router.cancel()
        router.accept(packet(MotionEvent.ACTION_HOVER_MOVE, 1, finger(1).copy(tool = PointerTool.Pen)))
        assertEquals(1, sink.hover); assertEquals(0, sink.started)
    }
    @Test fun genericProfilePreservesHistoricalRawPressureTiltButtonsAndCancel() {
        val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 4; toolType = MotionEvent.TOOL_TYPE_STYLUS })
        fun coordinates(pressure: Float) = arrayOf(MotionEvent.PointerCoords().apply {
            x = 12f; y = 24f; this.pressure = pressure
            setAxisValue(MotionEvent.AXIS_TILT, .25f); setAxisValue(MotionEvent.AXIS_ORIENTATION, -.5f)
        })
        val event = MotionEvent.obtain(100, 100, MotionEvent.ACTION_MOVE, 1, properties, coordinates(.3f),
            0, MotionEvent.BUTTON_STYLUS_PRIMARY, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
        try {
            event.addBatch(110, coordinates(.7f), 0)
            val packet = StylusInput().snapshot(event)
            assertEquals(2, packet.frames.size)
            assertEquals(.3f, packet.frames.first().pointers.single().rawPressure)
            assertEquals(.7f, packet.frames.last().pointers.single().pressure)
            assertEquals(.25f, packet.frames.last().pointers.single().tilt)
            assertEquals(-.5f, packet.frames.last().pointers.single().orientation)
            assertTrue(packet.capabilities.tilt); assertFalse(packet.capabilities.samsungActions)
            assertEquals(MotionEvent.BUTTON_STYLUS_PRIMARY, packet.frames.last().pointers.single().buttons)
        } finally { event.recycle() }
    }
    @Test fun curveControlsStayMonotonicWithoutChangingTheirEndpoints() {
        var curve = PressureCurve()
        for (control in 0..3) for (value in listOf(-2f, 2f, .4f, .9f, .1f)) {
            curve = curve.withControl(control, value)
            assertTrue(curve.x1 in 0f..curve.x2 && curve.x2 <= 1f)
            assertTrue(curve.y1 in 0f..curve.y2 && curve.y2 <= 1f)
        }
    }
}
