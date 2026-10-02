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
                // Activity creation loads vw_core. Missing/wrong-ABI artifacts fail before here.
                assertTrue(activity.window.decorView.isShown)
            }
        }
    }
}
