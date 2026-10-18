package com.visualworkbench.android

import android.view.InputDevice
import android.view.MotionEvent
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.android.input.StylusInput
import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Synthetic MotionEvents and explicit synthetic calibration. No real input,
 * device capability, S23 directional calibration, latency or injection claim. */
@RunWith(AndroidJUnit4::class)
class StylusTiltInstrumentedTest {
    @Test fun horizontalFloatAxesAndQuarterTurnProduceOnlyTheKnownCardinalComponent() {
        val properties = arrayOf(MotionEvent.PointerProperties().apply { id=1;toolType=MotionEvent.TOOL_TYPE_STYLUS })
        val halfPi=(Math.PI/2).toFloat()
        val coordinates=arrayOf(MotionEvent.PointerCoords().apply {
            x=20f;y=30f;pressure=.5f
            setAxisValue(MotionEvent.AXIS_TILT,halfPi);setAxisValue(MotionEvent.AXIS_ORIENTATION,halfPi)
        })
        val event=MotionEvent.obtain(100,100,MotionEvent.ACTION_DOWN,1,properties,coordinates,0,0,1f,1f,-1,0,InputDevice.SOURCE_STYLUS,0)
        try {
            val sample=StylusInput().snapshot(event).frames.single().pointers.single()
            val raw=sample.tilt to sample.orientation
            val signs=StylusTiltCalibration(StylusTiltSign.Direct,StylusTiltSign.Inverted)
            val right=checkNotNull(projectStylusTilt(sample.tilt?.toDouble(),sample.orientation?.toDouble(),0.0,signs))
            assertEquals(90.0,right.xDegrees,0.0);assertEquals(0.0,right.yDegrees,0.0)
            val down=checkNotNull(projectStylusTilt(sample.tilt?.toDouble(),sample.orientation?.toDouble(),halfPi.toDouble(),signs))
            assertEquals(0.0,down.xDegrees,0.0);assertEquals(90.0,down.yDegrees,0.0)
            assertEquals(raw,sample.tilt to sample.orientation)
        } finally { event.recycle() }
    }
    @Test fun reportedHistoricalAnglesStayRawWhileProjectionUsesAnExplicitCalibration() {
        val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 1; toolType = MotionEvent.TOOL_TYPE_STYLUS })
        fun coordinates(orientation: Float) = arrayOf(MotionEvent.PointerCoords().apply {
            x = 20f; y = 30f; pressure = .5f
            setAxisValue(MotionEvent.AXIS_TILT, (Math.PI / 6).toFloat())
            setAxisValue(MotionEvent.AXIS_ORIENTATION, orientation)
        })
        val event = MotionEvent.obtain(100, 100, MotionEvent.ACTION_MOVE, 1, properties,
            coordinates((Math.PI / 2).toFloat()), 0, 0, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
        try {
            event.addBatch(110, coordinates(0f), 0)
            val packet = StylusInput().snapshot(event)
            assertTrue(packet.capabilities.tilt); assertTrue(packet.capabilities.orientation)
            val samples = packet.frames.map { it.pointers.single() }
            assertEquals((Math.PI / 2).toFloat(), samples.first().orientation)
            assertEquals(0f, samples.last().orientation)
            val raw = samples.map { it.tilt to it.orientation }
            val signs = StylusTiltCalibration(StylusTiltSign.Direct, StylusTiltSign.Inverted)
            val right = checkNotNull(projectStylusTilt(samples.first().tilt?.toDouble(), samples.first().orientation?.toDouble(), 0.0, signs))
            val up = checkNotNull(projectStylusTilt(samples.last().tilt?.toDouble(), samples.last().orientation?.toDouble(), 0.0, signs))
            assertEquals(30.0, right.xDegrees, .00001); assertEquals(0.0, right.yDegrees, .00001)
            assertEquals(0.0, up.xDegrees, 0.0); assertEquals(-30.0, up.yDegrees, .00001)
            assertEquals(0u, right.rotationDegrees)
            assertEquals(raw, samples.map { it.tilt to it.orientation })
        } finally { event.recycle() }
    }
    @Test fun absentAxesRemainUnavailableAndCannotBeEnabledBySupplyingSigns() {
        val properties = arrayOf(MotionEvent.PointerProperties().apply { id = 1; toolType = MotionEvent.TOOL_TYPE_STYLUS })
        val coordinates = arrayOf(MotionEvent.PointerCoords().apply { x = 10f; y = 20f; pressure = .5f })
        val event = MotionEvent.obtain(100, 100, MotionEvent.ACTION_DOWN, 1, properties, coordinates,
            0, 0, 1f, 1f, -1, 0, InputDevice.SOURCE_STYLUS, 0)
        try {
            val packet = StylusInput().snapshot(event)
            val sample = packet.frames.single().pointers.single()
            assertFalse(packet.capabilities.tilt); assertFalse(packet.capabilities.orientation)
            assertNull(sample.tilt); assertNull(sample.orientation)
            assertNull(projectStylusTilt(sample.tilt?.toDouble(), sample.orientation?.toDouble(), 0.0,
                StylusTiltCalibration(StylusTiltSign.Direct, StylusTiltSign.Inverted)))
        } finally { event.recycle() }
    }
}
