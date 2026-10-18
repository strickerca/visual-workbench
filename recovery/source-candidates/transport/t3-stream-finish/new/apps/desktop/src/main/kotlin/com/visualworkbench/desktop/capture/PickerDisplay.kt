package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.PickerPreview
import com.visualworkbench.shared.Camera
import kotlinx.coroutines.CancellationException

/** Editor visibility and capture permission are separate outcomes. A denied
 * affinity operation must never strand an otherwise usable hidden editor. */
internal enum class PickerProtection { Pending, Ready, Unavailable;
    val editorVisible: Boolean get() = this != Pending
    val captureAllowed: Boolean get() = this == Ready
}

internal suspend fun initializePickerProtection(exclude: suspend () -> Unit): PickerProtection =
    try { exclude(); PickerProtection.Ready }
    catch (error: CancellationException) { throw error }
    catch (_: Exception) { PickerProtection.Unavailable }

/** Exact float transform used by the actual Canvas draw, without an integer
 * destination rectangle. The camera maps ORIGINAL pixel edges; the bitmap is
 * only a bounded display derivative and may have a different aspect rounding. */
internal data class PickerDisplayTransform(val left: Float, val top: Float, val scaleX: Float, val scaleY: Float)
internal fun pickerDisplayTransform(source: PickerPreview, camera: Camera): PickerDisplayTransform {
    require(source.sourceWidth > 0u && source.sourceHeight > 0u && source.width in 1u..1024u && source.height in 1u..1024u)
    require(camera.rotation == 0.0 && camera.scale.isFinite() && camera.scale > 0.0)
    require(listOf(camera.center.x, camera.center.y, camera.viewportWidth, camera.viewportHeight).all { it.isFinite() })
    require(camera.viewportWidth > 0.0 && camera.viewportHeight > 0.0)
    return PickerDisplayTransform(
        (camera.viewportWidth / 2 - camera.center.x * camera.scale).toFloat(),
        (camera.viewportHeight / 2 - camera.center.y * camera.scale).toFloat(),
        (source.sourceWidth.toDouble() * camera.scale / source.width.toDouble()).toFloat(),
        (source.sourceHeight.toDouble() * camera.scale / source.height.toDouble()).toFloat()
    ).also { require(listOf(it.left, it.top, it.scaleX, it.scaleY).all(Float::isFinite) && it.scaleX > 0f && it.scaleY > 0f) }
}
