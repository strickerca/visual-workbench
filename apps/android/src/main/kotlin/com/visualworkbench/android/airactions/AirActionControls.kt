package com.visualworkbench.android.airactions

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.Text
import androidx.compose.material.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.shared.AirActionCommand

/** Visible controls use the same command guard and serial completion as keys;
 * hardware availability is deliberately irrelevant to these buttons. */
@Composable
internal fun AirActionControls(editor: EditorController) {
    var expanded by remember(editor) { mutableStateOf(false) }
    Column {
        TextButton(onClick = { expanded = !expanded }, modifier = Modifier.heightIn(min = 48.dp)) {
            Text(if (expanded) "Hide pen shortcuts" else "Pen shortcuts")
        }
        if (expanded) {
            Text(if (editor.remoteOpen) "Destination: current PC editor. Only its verified Undo and Redo are available."
                else "Destination: this document. These controls also work without Air Actions.")
            Row(Modifier.horizontalScroll(rememberScrollState())) {
                AirActionCommand.entries.forEach { command ->
                    TextButton(onClick = { editor.airControl(command) }, enabled = editor.airCommandEnabled(command),
                        modifier = Modifier.heightIn(min = 48.dp)) { Text(command.label) }
                }
            }
        }
    }
}
