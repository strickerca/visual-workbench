package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.PickerPreview
import com.visualworkbench.shared.Camera
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

/** Editor visibility and capture permission are separate outcomes. A denied
 * affinity operation must never strand an otherwise usable hidden editor. */
internal enum class PickerProtection { Pending, Ready, Unavailable;
    val editorVisible: Boolean get() = this != Pending
    val captureAllowed: Boolean get() = this == Ready
}

internal suspend fun initializePickerProtection(exclude: suspend () -> Unit): PickerProtection =
    try { exclude(); currentCoroutineContext().ensureActive(); PickerProtection.Ready }
    catch (error: CancellationException) { throw error }
    catch (_: Exception) { PickerProtection.Unavailable }

/** Exact float transform used by the actual Canvas draw, without an integer
 * destination rectangle. The camera maps ORIGINAL pixel edges; the bitmap is
 * only a bounded display derivative and may have a different aspect rounding. */
internal data class PickerDisplayTransform(val left: Float, val top: Float, val scaleX: Float, val scaleY: Float)
internal fun pickerDisplayTransform(source: PickerPreview, camera: Camera): PickerDisplayTransform {
    require(source.sourceWidth in 1u..32768u && source.sourceHeight in 1u..32768u && source.width in 1u..1024u && source.height in 1u..1024u)
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


/** Refuse malformed derivatives before allocating a second bitmap copy. */
internal fun validatePickerPreview(source: PickerPreview) {
    require(source.sourceAssetId.isNotEmpty() && source.sourceAssetId.length <= 256)
    require(source.sourceWidth in 1u..32768u && source.sourceHeight in 1u..32768u)
    require(source.width in 1u..1024u && source.height in 1u..1024u)
    require(source.width <= source.sourceWidth && source.height <= source.sourceHeight)
    require(source.rgba.size == source.width.toInt() * source.height.toInt() * 4)
}

internal fun pickerArgb(source: PickerPreview): IntArray {
    validatePickerPreview(source)
    val w = source.width.toInt(); val h = source.height.toInt()
    return IntArray(w*h) { i -> val p=i*4; ((source.rgba[p+3].toInt() and 255) shl 24) or
        ((source.rgba[p].toInt() and 255) shl 16) or ((source.rgba[p+1].toInt() and 255) shl 8) or (source.rgba[p+2].toInt() and 255) }
}


/** Bounded in-memory PNG encoding; avoids ByteArrayOutputStream growth and
 * ImageIO's default disk cache. This is a display derivative, never an original. */
internal class PickerPngBuffer(maxBytes: Int = 8 * 1024 * 1024) : java.io.OutputStream() {
    init { require(maxBytes in 1..8 * 1024 * 1024) }
    private val buffer = ByteArray(maxBytes)
    private var size = 0
    override fun write(value: Int) {
        require(size < buffer.size)
        buffer[size++] = value.toByte()
    }
    override fun write(bytes: ByteArray, offset: Int, length: Int) {
        require(offset >= 0 && length >= 0 && offset <= bytes.size - length && length <= buffer.size - size)
        bytes.copyInto(buffer, size, offset, offset + length)
        size += length
    }
    fun bytes(): ByteArray = buffer.copyOf(size)
}
