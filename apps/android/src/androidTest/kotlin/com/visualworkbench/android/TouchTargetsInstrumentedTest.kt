package com.visualworkbench.android

import android.graphics.Rect
import android.os.SystemClock
import android.util.Log
import android.view.accessibility.AccessibilityNodeInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Real accessible project-screen hit bounds in both themes. No screenshots. */
@RunWith(AndroidJUnit4::class)
class TouchTargetsInstrumentedTest {
    @Test fun projectActionsExposeAtLeastFortyEightDpTouchTargetsInBothThemes() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            val startupDeadline = SystemClock.uptimeMillis() + 15_000
            var ready = false
            while (!ready && SystemClock.uptimeMillis() < startupDeadline) {
                scenario.onActivity { ready = !it.editor.busy }
                if (!ready) SystemClock.sleep(25)
            }
            assertTrue("Project initialization did not finish", ready)
            var originalDark = true
            scenario.onActivity { originalDark = it.editor.preferences.darkTheme }
            try {
                for (dark in listOf(false, true)) {
                    scenario.onActivity { it.editor.settings(it.editor.preferences.copy(darkTheme = dark)) }
                    val required = setOf("Photos", "Browse files", "Take photo", "New canvas", "Settings", "Diagnostics", "Pair a computer")
                    val deadline = SystemClock.uptimeMillis() + 15_000
                    var found = emptyMap<String, Rect>()
                    while (SystemClock.uptimeMillis() < deadline) {
                        val root = instrumentation.uiAutomation.rootInActiveWindow
                        if (root != null && root.packageName?.toString() == instrumentation.targetContext.packageName) {
                            val targets = LinkedHashMap<String, Rect>()
                            fun labels(node: AccessibilityNodeInfo): Set<String> {
                                val result = mutableSetOf<String>()
                                node.text?.toString()?.let(result::add); node.contentDescription?.toString()?.let(result::add)
                                for (i in 0 until node.childCount) node.getChild(i)?.let { result.addAll(labels(it)) }
                                return result
                            }
                            fun scan(node: AccessibilityNodeInfo) {
                                if (node.isVisibleToUser && node.isClickable) {
                                    for (label in labels(node).intersect(required)) targets[label] = Rect().also(node::getBoundsInScreen)
                                }
                                for (i in 0 until node.childCount) node.getChild(i)?.let(::scan)
                            }
                            scan(root); found = targets
                            if (found.keys.containsAll(required)) break
                        }
                        SystemClock.sleep(50)
                    }
                    assertEquals("Foreground project actions were not available", required, found.keys)
                    val density = instrumentation.targetContext.resources.displayMetrics.density
                    for ((name, bounds) in found) {
                        val width = bounds.width() / density; val height = bounds.height() / density
                        Log.i("VW_TOUCH_TARGET", "theme=${if (dark) "dark" else "light"} control=$name width_dp=$width height_dp=$height")
                        assertTrue("$name width $width", width >= 47.5f)
                        assertTrue("$name height $height", height >= 47.5f)
                    }
                }
            } finally { scenario.onActivity { it.editor.settings(it.editor.preferences.copy(darkTheme = originalDark)) } }
        }
    }
}
