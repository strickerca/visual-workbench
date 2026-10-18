package com.visualworkbench.shared

import kotlin.test.*

class RemotePenAxesTest {
    @Test fun absentAxesAndMissingCalibrationKeepPressureWithoutAdvertisingTilt() {
        assertEquals(RemoteTiltStatus.Unavailable, remotePenAxes(null, 0.0, 0.0, null).status)
        val value = remotePenAxes(.5, .2, 0.0, null)
        assertEquals(RemoteTiltStatus.Uncalibrated, value.status)
        assertEquals(1u, value.nativePenMask)
        assertEquals(0, value.tiltX); assertEquals(0, value.tiltY)
    }
    @Test fun calibratedProjectionAdvertisesOnlyTheReportedTiltPairAndNeverTwist() {
        val signs = StylusTiltCalibration(StylusTiltSign.Direct, StylusTiltSign.Inverted)
        val value = remotePenAxes(Math.PI / 6, Math.PI / 4, Math.PI / 2, signs)
        assertEquals(RemoteTiltStatus.Calibrated, value.status)
        // atan(tan(30 degrees)/sqrt(2)), not the old linear 30/sqrt(2).
        assertEquals(22, value.tiltX); assertEquals(22, value.tiltY)
        assertEquals(13u, value.nativePenMask)
        assertEquals(0u, value.nativePenMask and 2u)
    }
}
