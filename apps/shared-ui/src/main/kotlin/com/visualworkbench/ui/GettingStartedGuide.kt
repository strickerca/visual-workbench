package com.visualworkbench.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.width
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

/** One step of the first-run guide: what to do, and where to find it. */
internal data class GuideStep(val title: String, val detail: String)

/** The whole loop in four steps, in the words each app uses on its own
 * buttons and menus. Shown where a first-time user lands: the empty project
 * list on the phone and the empty window on the PC. */
internal fun gettingStartedSteps(onPhone: Boolean): List<GuideStep> = if (onPhone) listOf(
    GuideStep("Open an image", "Choose Photos, Take photo or Browse files, or start a New canvas."),
    GuideStep("Mark it up", "The pen draws; your fingers move and zoom. Shapes, text and numbered markers are in the tools. Every mark can be undone."),
    GuideStep("Use it with your computer", "Pair a computer once. After that, receive the project that is open on the computer and work on it from both."),
    GuideStep("Hand it off", "Export a marked or a clean image when you are done. The original stays untouched."),
) else listOf(
    GuideStep("Open an image", "Import a file (Ctrl+I) or drop an image into this window."),
    GuideStep("Mark it up", "Pick the pen, a shape, text or a numbered marker on the left. Every mark can be undone (Ctrl+Z)."),
    GuideStep("Use it with your phone", "Pair devices once (Ctrl+P). After that, share the open project with the phone and draw on it with the pen."),
    GuideStep("Hand it off", "Export the image (Ctrl+E) and attach it for your AI assistant, or publish it under Workbench → Local agents and Compare."),
)

@Composable
internal fun GettingStartedGuide(onPhone: Boolean, modifier: Modifier = Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        gettingStartedSteps(onPhone).forEachIndexed { index, step ->
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("${index + 1}", Modifier.width(18.dp), style = MaterialTheme.typography.subtitle1,
                    fontWeight = FontWeight.Bold, color = MaterialTheme.colors.primary)
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(step.title, style = MaterialTheme.typography.subtitle1, fontWeight = FontWeight.SemiBold)
                    Text(step.detail, style = MaterialTheme.typography.body2)
                }
            }
        }
    }
}
