package com.visualworkbench.android

import android.content.Context
import android.content.ContextWrapper
import android.content.SharedPreferences
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID

@RunWith(AndroidJUnit4::class)
class EditorPreferencesInstrumentedTest {
    @Test fun perBrushSettingsRoundTripAndCorruptPreferencesUseSafeDefaults() = runBlocking {
        val base = InstrumentationRegistry.getInstrumentation().targetContext
        val prefix = "editor-test-${UUID.randomUUID()}-"
        val context = object : ContextWrapper(base) {
            override fun getApplicationContext(): Context = this
            override fun getSharedPreferences(name: String, mode: Int): SharedPreferences = base.getSharedPreferences(prefix + name, mode)
        }
        try {
            val store = EditorPreferencesStore(context)
            val settings = EditorPreferences(drawWithFinger = true, leftHanded = true, darkTheme = false, reducedMotion = true, railOffset = .7f)
                .withBrush(EditorTool.Pen, BrushPreference(8.25, 0xff123456, .75f, PressureCurve(.1f, .02f, .6f, .4f)))
                .withBrush(EditorTool.Highlighter, BrushPreference(28.0, 0x66ffff00, .25f, PressureCurve(.2f, .4f, .8f, .9f)))
            store.write(settings)
            assertEquals(settings, EditorPreferencesStore(context).read())
            assertNotEquals(settings.brush(EditorTool.Pen), settings.brush(EditorTool.Highlighter))
            context.getSharedPreferences("editor-preferences", Context.MODE_PRIVATE).edit().putString("settings", "{\"Pen\":{\"width\":-1}}").commit()
            assertEquals(EditorPreferences(), store.read())
            context.getSharedPreferences("editor-preferences", Context.MODE_PRIVATE).edit().putString("settings", "x".repeat(16_385)).commit()
            assertEquals(EditorPreferences(), store.read())
        } finally { assertTrue(base.deleteSharedPreferences(prefix + "editor-preferences")) }
    }
}
