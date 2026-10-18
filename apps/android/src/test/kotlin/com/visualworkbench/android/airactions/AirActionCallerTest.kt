package com.visualworkbench.android.airactions

import com.visualworkbench.android.editor.BrushPreference
import com.visualworkbench.android.editor.EditorPreferences
import com.visualworkbench.android.editor.EditorTool
import com.visualworkbench.shared.AirActionBindings
import com.visualworkbench.shared.AirActionCommand
import com.visualworkbench.shared.AirActionGesture
import com.visualworkbench.shared.AirActionBinding
import com.visualworkbench.shared.AirActionContext
import com.visualworkbench.shared.AirActionKeyEvent
import com.visualworkbench.shared.AirActionKeyPhase
import com.visualworkbench.shared.AirActionRouter
import com.visualworkbench.shared.CommandDispatcher
import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.Point
import com.visualworkbench.shared.RemoteEditorAction
import org.junit.Assert.*
import org.junit.Test
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel

class AirActionCallerTest {
    @Test fun legacy_missing_entries_keep_defaults() {
        assertEquals(AirActionBindings(), decodeAirActionPreferences(emptyMap()))
        assertEquals(AirActionCommand.Undo, decodeAirActionPreferences(mapOf("futureGesture" to "Redo")).command(AirActionGesture.SwipeLeft))
    }
    @Test fun remapped_and_explicit_disabled_slots_round_trip_by_stable_name() {
        val bindings = AirActionBindings().remap(AirActionGesture.SingleClick, null)
            .remap(AirActionGesture.SwipeRight, AirActionCommand.LensMagnificationDown)
        val saved = encodeAirActionPreferences(bindings)
        assertTrue(saved.containsKey("SingleClick")); assertNull(saved["SingleClick"])
        assertEquals("LensMagnificationDown", saved["SwipeRight"])
        assertEquals(bindings, decodeAirActionPreferences(saved))
    }
    @Test fun unknown_or_malformed_command_disables_only_its_known_slot() {
        for (raw in listOf("FutureCommand", "Send", "Apply", "0", 0, true, listOf("Undo"), null)) {
            val bindings = decodeAirActionPreferences(mapOf("SwipeLeft" to raw))
            assertNull(bindings.command(AirActionGesture.SwipeLeft))
            assertEquals(AirActionCommand.Redo, bindings.command(AirActionGesture.SwipeRight))
        }
    }
    @Test fun preference_copy_retains_unrelated_settings_and_brushes() {
        val original = EditorPreferences(drawWithFinger = true, leftHanded = true, darkTheme = false,
            reducedMotion = true, railOffset = .75f,
            brushes = mapOf(EditorTool.Pen to BrushPreference(width = 23.0, colorArgb = 0xff123456)))
        val changed = original.copy(airActions = original.airActions.remap(AirActionGesture.DoubleClick, null))
        assertEquals(original, changed.copy(airActions = original.airActions))
        assertEquals(23.0, changed.brush(EditorTool.Pen).width, 0.0)
        assertNull(decodeAirActionPreferences(encodeAirActionPreferences(changed.airActions)).command(AirActionGesture.DoubleClick))
    }
    @Test fun persisted_values_contain_only_eight_binding_names() {
        val encoded = encodeAirActionPreferences(AirActionBindings())
        assertEquals(AirActionGesture.entries.map { it.name }.toSet(), encoded.keys)
        assertTrue(encoded.values.all { it in AirActionCommand.entries.map { command -> command.name } })
        assertFalse(encoded.keys.any { it.contains("connected", ignoreCase = true) || it.contains("support", ignoreCase = true) })
    }

    private val before = Camera(Point(112.25, 92.5), 3.75, .314, 1024.0, 768.0)
    private val fitted = before.copy(center = Point(400.0, 300.0), scale = .92, rotation = 0.0)
    private val owner = AirActionViewOwner(11, "document", 2u, 1024.0, 768.0)

    @Test fun fit_restore_returns_the_exact_borrowed_camera_once() {
        val restore = AirActionViewRestore()
        restore.remember(owner, before, fitted)
        assertSame(before, restore.take(owner, fitted))
        assertNull(restore.take(owner, fitted))
    }
    @Test fun replacement_project_document_page_or_viewport_refuses_and_discards_restore() {
        for (changed in listOf(owner.copy(projectEpoch = 12), owner.copy(documentId = "replacement"),
            owner.copy(page = 3u), owner.copy(width = 768.0), owner.copy(height = 1024.0))) {
            val restore = AirActionViewRestore(); restore.remember(owner, before, fitted)
            assertNull(restore.take(changed, fitted)); assertNull(restore.take(owner, fitted))
        }
    }
    @Test fun intervening_navigation_cannot_restore_a_stale_camera() {
        val restore = AirActionViewRestore(); restore.remember(owner, before, fitted)
        assertNull(restore.take(owner, fitted.copy(center = Point(0.0, 0.0))))
        assertNull(restore.take(owner, fitted))
    }
    @Test fun explicit_invalidation_prevents_away_and_back_restoration() {
        val restore = AirActionViewRestore(); restore.remember(owner, before, fitted)
        restore.clear(); assertNull(restore.take(owner, fitted))
    }
    @Test fun remote_mapping_has_no_local_undo_fallback_or_guessed_view_command() {
        for (command in AirActionCommand.entries) assertEquals(when (command) {
            AirActionCommand.Undo -> RemoteEditorAction.Undo
            AirActionCommand.Redo -> RemoteEditorAction.Redo
            else -> null
        }, remoteAirAction(command))
    }

    private class BarrelFixture : AutoCloseable {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val dispatcher = CommandDispatcher(scope)
        var time = 1_000L
        var calls = 0
        val observed = AirActionBarrelCorrelation()
        val router = AirActionRouter(dispatcher, { time }, { AirActionBinding(1, AirActionContext.Canvas) },
            { _, _ -> true }, { _, _ -> calls++ })
        init { router.configure(AirActionBindings(), true); time++ }
        fun barrel(down: Boolean) { observed.observe(down, time); router.barrel(down, time) }
        fun pair() {
            val down = time
            router.key(AirActionKeyEvent(AirActionGesture.SwipeLeft, AirActionKeyPhase.Down, 1, down, time))
            time++
            router.key(AirActionKeyEvent(AirActionGesture.SwipeLeft, AirActionKeyPhase.Up, 1, down, time))
            time += AirActionRouter.ResolutionMs; router.advance()
        }
        override fun close() { router.close(); dispatcher.close(); scope.cancel() }
    }
    @Test fun canvas_interruption_preserves_observed_physical_barrel_hold() = BarrelFixture().use { f ->
        f.barrel(true); f.router.interrupt(); f.observed.restore(f.router, f.time); f.time++
        f.pair(); assertEquals(0, f.calls)
    }
    @Test fun canvas_interruption_preserves_barrel_release_correlation_window() = BarrelFixture().use { f ->
        f.barrel(true); f.time += 10; f.barrel(false)
        f.router.interrupt(); f.observed.restore(f.router, f.time); f.time++
        f.pair(); assertEquals(0, f.calls)
        f.time++; f.pair(); assertEquals(1, f.calls)
    }
    @Test fun full_input_owner_loss_clears_old_barrel_observation() = BarrelFixture().use { f ->
        f.barrel(true); f.observed.clear(); f.router.interrupt(); f.observed.restore(f.router, f.time); f.time++
        f.pair(); assertEquals(1, f.calls)
    }
}
