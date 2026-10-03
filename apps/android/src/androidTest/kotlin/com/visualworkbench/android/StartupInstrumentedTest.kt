package com.visualworkbench.android

import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Exercises the built APK and its packaged native library on real hardware. */
@RunWith(AndroidJUnit4::class)
class StartupInstrumentedTest {
    @Test
    fun activityLoadsNativeCoreAndShowsItsWindow() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            assertEquals(Lifecycle.State.RESUMED, scenario.state)
            scenario.onActivity { activity ->
                // Invoke the packaged core explicitly, regardless of whether
                // local identity/project initialization has completed yet.
                assertTrue(activity.editor.core.newId(System.currentTimeMillis().toULong()).matches(Regex("[0-9a-f-]{36}")))
                assertTrue(activity.window.decorView.isShown)
            }
        }
    }
}
