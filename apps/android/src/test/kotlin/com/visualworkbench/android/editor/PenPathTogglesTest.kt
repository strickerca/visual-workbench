package com.visualworkbench.android.editor

import org.junit.Assert.*
import org.junit.Test

class PenPathTogglesTest {
    /** The owner switches each one on during a measured session; nothing ships on. */
    @Test fun every_developer_switch_is_off_by_default() {
        val defaults = PenPathToggles()
        assertFalse(defaults.anyOn)
        for (toggle in PenPathToggle.entries) assertFalse(toggle.name, defaults[toggle])
        assertEquals(defaults, EditorPreferences().penPath)
        for (toggle in PenPathToggle.entries) assertFalse(toggle.name, EditorPreferences().penPath[toggle])
        assertTrue(defaults.encode().isEmpty())
    }
    @Test fun missing_unknown_or_malformed_stored_values_leave_every_switch_off() {
        assertEquals(PenPathToggles(), PenPathToggles.decode(emptyMap()))
        for (raw in listOf(null, false, "true", 1, 1.0, listOf(true), mapOf("on" to true))) {
            val stored = PenPathToggle.entries.associate { it.key to raw }
            assertEquals("raw=$raw", PenPathToggles(), PenPathToggles.decode(stored))
        }
        assertEquals(PenPathToggles(), PenPathToggles.decode(mapOf("notASwitch" to true, "" to true)))
    }
    /** The canvas switches whose behaviour became the app's only path are
     * gone, and no canvas switch is left. A phone that had them switched on
     * keeps its other settings: the stored names are read past, and the next
     * save leaves them out. */
    @Test fun a_stored_switch_that_no_longer_exists_is_ignored_without_error() {
        val removed = listOf("canvasQuickRefresh", "canvasInkBeforeSave", "canvasInkOnly")
        assertTrue(PenPathToggle.entries.none { it.key in removed })
        assertTrue(PenPathToggle.entries.none { it.key.startsWith("canvas") })
        assertEquals(PenPathToggles(), PenPathToggles.decode(removed.associateWith { true }))
        val kept = PenPathToggle.RemoteUnbufferedPen
        val decoded = PenPathToggles.decode(removed.associateWith { true } + (kept.key to true))
        assertEquals(PenPathToggles().with(kept, true), decoded)
        assertEquals(mapOf(kept.key to true), decoded.encode())
    }
    @Test fun each_switch_round_trips_alone_by_its_stable_key_without_touching_the_others() {
        assertEquals(PenPathToggle.entries.size, PenPathToggle.entries.map { it.key }.toSet().size)
        for (toggle in PenPathToggle.entries) {
            assertTrue(toggle.key.matches(Regex("[a-z][A-Za-z0-9]{2,40}")))
            assertTrue(toggle.title.isNotBlank() && toggle.effect.isNotBlank())
            val one = PenPathToggles().with(toggle, true)
            assertTrue(one[toggle]); assertTrue(one.anyOn)
            for (other in PenPathToggle.entries) if (other != toggle) assertFalse(one[other])
            assertEquals(mapOf(toggle.key to true), one.encode())
            assertEquals(one, PenPathToggles.decode(one.encode()))
            assertEquals(PenPathToggles(), one.with(toggle, false))
        }
        val all = PenPathToggle.entries.fold(PenPathToggles()) { set, toggle -> set.with(toggle, true) }
        assertEquals(all, PenPathToggles.decode(all.encode()))
    }
    @Test fun changing_a_switch_keeps_unrelated_settings_and_brushes() {
        val original = EditorPreferences(drawWithFinger = true, leftHanded = true, darkTheme = false, reducedMotion = true, railOffset = .75f,
            brushes = mapOf(EditorTool.Pen to BrushPreference(width = 23.0, colorArgb = 0xff123456)))
        val toggle = PenPathToggle.entries.first()
        val changed = original.copy(penPath = original.penPath.with(toggle, true))
        assertTrue(changed.penPath[toggle])
        assertEquals(original, changed.copy(penPath = PenPathToggles()))
    }
}
