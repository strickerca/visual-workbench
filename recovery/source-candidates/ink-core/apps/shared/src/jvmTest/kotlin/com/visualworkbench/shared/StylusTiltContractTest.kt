package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

/** Actual generated binding contract with synthetic signs, never device evidence. */
class StylusTiltContractTest {
    private val signs = StylusTiltCalibration(StylusTiltSign.Direct, StylusTiltSign.Inverted)

    @Test fun optionalHardwareAxesAndMissingCalibrationStayDistinctAcrossBinding() {
        assertNull(projectStylusTilt(null, null, 0.0, null))
        assertNull(projectStylusTilt(.5, null, 0.0, signs))
        val refused = assertThrows(StylusTiltFailure::class.java) {
            projectStylusTilt(.5, 0.0, 0.0, null)
        }
        assertEquals(StylusTiltFailureKind.Uncalibrated, refused.kind)
    }
    @Test fun knownCardinalDirectionsAndViewRotationHaveDegreeResultsAndNoTwist() {
        val right = checkNotNull(projectStylusTilt(Math.PI / 6.0, 0.0, Math.PI / 2.0, signs))
        assertEquals(30.0, right.xDegrees, 1e-10)
        assertEquals(0.0, right.yDegrees, 0.0)
        assertEquals(0u, right.rotationDegrees)
        val bottom = checkNotNull(projectStylusTilt(Math.PI / 6.0, Math.PI, 0.0, signs))
        assertEquals(0.0, bottom.xDegrees, 0.0)
        assertEquals(30.0, bottom.yDegrees, 1e-10)
    }
    @Test fun malformedAnglesAreTypedFailuresInsteadOfDroppedOrInventedMeasurements() {
        for (angle in listOf(Double.NaN, Double.POSITIVE_INFINITY, -.1, 2.0)) {
            assertEquals(StylusTiltFailureKind.Invalid, assertThrows(StylusTiltFailure::class.java) {
                projectStylusTilt(angle, 0.0, 0.0, signs)
            }.kind)
        }
        assertEquals(StylusTiltFailureKind.Invalid, assertThrows(StylusTiltFailure::class.java) {
            projectStylusTilt(null, null, Double.NaN, null)
        }.kind)
    }
}
