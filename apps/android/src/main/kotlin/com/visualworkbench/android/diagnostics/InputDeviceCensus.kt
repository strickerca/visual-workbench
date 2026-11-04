package com.visualworkbench.android.diagnostics

import android.view.InputDevice

/** Capability-only input-device census shared by trace and physical-gesture
 * journals. Device names, descriptors, vendor/product IDs and serials are
 * deliberately never read (StylusInput privacy rule). */
internal data class InputDeviceRange(
    val axis:Int,val source:Int,val min:Float,val max:Float,val flat:Float,val fuzz:Float,val resolution:Float,
)
internal data class InputDeviceCapability(
    val deviceId:Int,val sources:Int,val declaresStylus:Boolean,val virtual:Boolean,val external:Boolean,
    val ranges:List<InputDeviceRange>,
)

internal fun inputDeviceCapability(device:InputDevice):InputDeviceCapability = InputDeviceCapability(
    device.id,device.sources,device.supportsSource(InputDevice.SOURCE_STYLUS),device.isVirtual,device.isExternal,
    device.motionRanges.map{range->
        InputDeviceRange(range.axis,range.source,range.min,range.max,range.flat,range.fuzz,range.resolution)
    },
)
