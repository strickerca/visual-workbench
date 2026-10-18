package com.visualworkbench.android.airactions

/** Observation only. No coordinates, pressure, native input, ACK or grant. */
internal enum class AirRemoteMotionPhase { Down, PointerDown, Move, Up, PointerUp, Hover, Leave, Cancel, Other }
internal data class AirRemoteMotionPointer(val id: Int, val stylus: Boolean)
internal data class AirRemoteMotionFrame(val phase: AirRemoteMotionPhase, val pointer: Int,
    val pointers: List<AirRemoteMotionPointer>, val primaryBarrel: Boolean, val timeMs: Long,
    val canceled: Boolean = false)

/** One Dialog registration. Exact reference identity and permanent detach prevent
 * an obsolete Window callback from changing a replacement owner's admission.
 * The existing router/serial dispatcher is retired before contact can end. */
internal class AirActionRemoteMotion(
    private val remote: Any,
    private val palette: Any,
    private val retire: (clearBarrel: Boolean) -> Unit,
    private val barrel: (Boolean, Long) -> Unit,
) {
    private var focused = false
    private var closed = false
    private val contacts = mutableSetOf<Int>()
    val contactActive: Boolean get() = contacts.isNotEmpty()

    private fun owns(remote: Any, palette: Any): Boolean = !closed && this.remote === remote && this.palette === palette

    fun focus(remote: Any, palette: Any, active: Boolean): Boolean {
        if (!owns(remote, palette) || focused == active) return false
        retire(true)
        contacts.clear()
        focused = active
        return true
    }

    fun reset(remote: Any, palette: Any) {
        if (!owns(remote, palette)) return
        retire(true)
        contacts.clear()
    }

    fun detach(remote: Any, palette: Any) {
        if (!owns(remote, palette)) return
        retire(true)
        contacts.clear()
        focused = false
        closed = true
    }

    fun observe(remote: Any, palette: Any, frame: AirRemoteMotionFrame) {
        if (!owns(remote, palette) || !focused) return
        if (frame.timeMs < 0 || frame.pointers.size > 16 || frame.phase == AirRemoteMotionPhase.Cancel ||
            (frame.canceled && frame.phase != AirRemoteMotionPhase.PointerUp)) {
            reset(remote, palette)
            return
        }
        val pens = frame.pointers.filter { it.stylus }.map { it.id }.toSet()
        val changedPen = frame.pointer in pens || frame.pointer in contacts
        when (frame.phase) {
            AirRemoteMotionPhase.Down -> {
                if (changedPen || contactActive) {
                    retire(false)
                    contacts.clear()
                    if (frame.pointer in pens) contacts += frame.pointer
                }
            }
            AirRemoteMotionPhase.PointerDown -> if (frame.pointer in pens) {
                retire(false)
                contacts.retainAll(pens)
                contacts += frame.pointer
            }
            AirRemoteMotionPhase.Move -> if (pens.isNotEmpty() && contacts != pens) {
                // Missing Down still blocks Air until an observed terminal or
                // full owner reset. Native input independently decides admission.
                retire(false)
                contacts.clear()
                contacts += pens
            }
            AirRemoteMotionPhase.Up -> if (changedPen || contactActive) {
                retire(false)
                contacts.clear()
            }
            AirRemoteMotionPhase.PointerUp -> if (changedPen) {
                retire(frame.canceled)
                contacts.remove(frame.pointer)
            }
            AirRemoteMotionPhase.Hover, AirRemoteMotionPhase.Leave -> if (contactActive) {
                retire(false)
                contacts.clear()
            }
            AirRemoteMotionPhase.Cancel, AirRemoteMotionPhase.Other -> Unit
        }
        if (pens.isNotEmpty() || changedPen) barrel(frame.primaryBarrel, frame.timeMs)
    }
}

/** Observe first, then preserve the exact event, delegate call count, result and
 * delegate exception. A failed observer fences Air and cannot consume motion. */
internal fun <T> observeAirRemoteMotionBeforeForward(event: T, observe: (T) -> Unit,
    failed: () -> Unit, forward: (T) -> Boolean): Boolean {
    try { observe(event) }
    catch (_: Exception) { try { failed() } catch (_: Exception) { /* Air failure cannot replace the original motion result. */ } }
    return forward(event)
}
