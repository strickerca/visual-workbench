package com.visualworkbench.desktop

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Surface
import androidx.compose.material.darkColors
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.visualworkbench.ui.GettingStartedGuide
import com.visualworkbench.ui.gettingStartedSteps
import org.jetbrains.skia.EncodedImageFormat
import org.junit.Assert.*
import org.junit.Test
import java.io.File

class GettingStartedGuideTest {
    @Test fun bothAppsGetFourShortStepsInTheirOwnWords() {
        val phone = gettingStartedSteps(onPhone = true); val pc = gettingStartedSteps(onPhone = false)
        for (steps in listOf(phone, pc)) {
            assertEquals(4, steps.size)
            assertTrue(steps.all { it.title.length in 5..40 && it.detail.length in 20..200 && it.detail.endsWith(".") })
        }
        assertEquals(phone.map { it.title.substringBefore(" with") }, pc.map { it.title.substringBefore(" with") })
        // The PC text names the shortcuts the menus show; the phone text names none.
        assertTrue(listOf("Ctrl+I", "Ctrl+Z", "Ctrl+P", "Ctrl+E").all { key -> pc.any { key in it.detail } })
        assertTrue(phone.none { "Ctrl" in it.detail })
    }

    /** Draws the guide off screen at the width of the PC's empty-state column.
     * With VW_SCREENSHOT_DIR set, the picture is also written there for a person
     * to look at; nothing is written otherwise. */
    @Test fun theGuideLaysOutInTheEmptyStateColumnAndDrawsSomething() {
        var size: IntSize? = null
        val scene = ImageComposeScene(420, 520, Density(1f)) {
            MaterialTheme(colors = darkColors()) {
                Surface(Modifier.fillMaxWidth()) {
                    GettingStartedGuide(onPhone = false, modifier = Modifier.fillMaxWidth().padding(24.dp).onGloballyPositioned { size = it.size })
                }
            }
        }
        try {
            val image = scene.render(0)
            try {
                val laid = checkNotNull(size)
                assertEquals(420 - 2 * 24, laid.width)
                assertTrue("guide is ${laid.height} px tall", laid.height in 150..520)
                val pixels = org.jetbrains.skia.Bitmap().also { check(it.allocN32Pixels(image.width, image.height)); check(image.readPixels(it)) }
                val colours = HashSet<Int>()
                for (y in 0 until image.height step 3) for (x in 0 until image.width step 3) colours += pixels.getColor(x, y)
                assertTrue("text was drawn", colours.size > 8)
                System.getenv("VW_SCREENSHOT_DIR")?.takeIf { it.isNotBlank() }?.let { folder ->
                    File(folder, "getting-started-pc.png").writeBytes(checkNotNull(image.encodeToData(EncodedImageFormat.PNG)).bytes)
                }
            } finally { image.close() }
        } finally { scene.close() }
    }
}
