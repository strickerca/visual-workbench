package com.visualworkbench.shared

/** Values supplied by a measured per-device directional calibration, never a
 * vendor guess. Omit calibration until the owner has provided those traces. */
public enum class StylusTiltSign { Direct, Inverted }
public data class StylusTiltCalibration(public val x: StylusTiltSign, public val y: StylusTiltSign)
public data class ProjectedStylusTilt(public val xDegrees: Double, public val yDegrees: Double,
    public val rotationDegrees: UInt)
public enum class StylusTiltFailureKind { Invalid, Uncalibrated }
public class StylusTiltFailure(public val kind: StylusTiltFailureKind, cause: Throwable? = null) : Exception(kind.name, cause)

/** Small synchronous libm projection using the existing core-library bootstrap.
 * It does not read a device, change settings, or inject input. The signed clockwise
 * rotation maps the input screen into the target frame; orientation is not twist.
 * Missing either hardware axis returns null. Reported axes without explicit
 * calibration throw Uncalibrated; no input API is invoked or authorized here. */
public expect fun projectStylusTilt(tilt: Double?, orientation: Double?, viewToFrameRotation: Double,
    calibration: StylusTiltCalibration?): ProjectedStylusTilt?
