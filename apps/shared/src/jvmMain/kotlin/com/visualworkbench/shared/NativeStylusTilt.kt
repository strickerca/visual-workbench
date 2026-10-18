package com.visualworkbench.shared

import com.visualworkbench.bindings.core.StylusTiltCalibration as NCalibration
import com.visualworkbench.bindings.core.StylusTiltException
import com.visualworkbench.bindings.core.StylusTiltSign as NSign

public actual fun projectStylusTilt(tilt: Double?, orientation: Double?, viewToFrameRotation: Double,
    calibration: StylusTiltCalibration?): ProjectedStylusTilt? {
    prepareCoreLibrary()
    return try {
        com.visualworkbench.bindings.core.projectStylusTilt(tilt, orientation, viewToFrameRotation,
            calibration?.let { NCalibration(it.x.native(), it.y.native()) })?.let {
            ProjectedStylusTilt(it.xDegrees, it.yDegrees, it.rotationDegrees)
        }
    } catch (failure: StylusTiltException) {
        throw StylusTiltFailure(when (failure) {
            is StylusTiltException.Invalid -> StylusTiltFailureKind.Invalid
            is StylusTiltException.Uncalibrated -> StylusTiltFailureKind.Uncalibrated
        }, failure)
    }
}
private fun StylusTiltSign.native(): NSign = when (this) {
    StylusTiltSign.Direct -> NSign.DIRECT
    StylusTiltSign.Inverted -> NSign.INVERTED
}
