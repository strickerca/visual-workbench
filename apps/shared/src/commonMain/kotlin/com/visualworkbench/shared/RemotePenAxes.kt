package com.visualworkbench.shared

import kotlin.math.roundToInt

public enum class RemoteTiltStatus { Unavailable, Uncalibrated, Calibrated }
public data class RemotePenAxes(public val tiltX: Int = 0, public val tiltY: Int = 0,
    public val nativePenMask: UInt = 1u, public val status: RemoteTiltStatus = RemoteTiltStatus.Unavailable)

/** Pressure remains available when tilt axes or measured per-device signs are
 * absent. Orientation is never advertised as barrel twist. Raw strokes retain
 * the original axes; only this remote input projection becomes Windows angles. */
public fun remotePenAxes(tilt: Double?, orientation: Double?, viewToFrameRotation: Double,
    calibration: StylusTiltCalibration?): RemotePenAxes {
    if (tilt == null || orientation == null) return RemotePenAxes()
    if (calibration == null) return RemotePenAxes(status = RemoteTiltStatus.Uncalibrated)
    val projected = projectStylusTilt(tilt, orientation, viewToFrameRotation, calibration)
        ?: return RemotePenAxes()
    return RemotePenAxes(projected.xDegrees.roundToInt(), projected.yDegrees.roundToInt(),
        nativePenMask = 13u, status = RemoteTiltStatus.Calibrated)
}
