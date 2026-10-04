package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.SwingPanel
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindow
import androidx.compose.ui.window.rememberDialogState
import java.awt.event.WindowAdapter
import java.awt.event.WindowEvent
import javax.swing.JPasswordField
import javax.swing.text.AbstractDocument
import javax.swing.text.AttributeSet
import javax.swing.text.DocumentFilter

/** Uses an ordinary non-saveable password component. No call reads the stored
 * secret into UI; only readiness/configured metadata is displayed. */
internal class AiPasswordField : JPasswordField() {
    init {
        accessibleContext.accessibleName = "Image provider API key"
        columns = 32
        (document as AbstractDocument).documentFilter = object : DocumentFilter() {
            override fun insertString(fb: FilterBypass, offset: Int, value: String?, attrs: AttributeSet?) =
                replace(fb, offset, 0, value, attrs)
            override fun replace(fb: FilterBypass, offset: Int, length: Int, value: String?, attrs: AttributeSet?) {
                val incoming = value.orEmpty()
                if (incoming.length > 2560 || fb.document.length - length + incoming.length > 2560 ||
                    incoming.any { it.code !in 33..126 }) return
                super.replace(fb, offset, length, incoming, attrs)
            }
        }
    }
    fun clearInput() { text = "" }
    fun takeBytes(): ByteArray? {
        val chars = password
        clearInput()
        return try {
            if (chars.size !in 1..2560 || chars.any { it.code !in 33..126 }) null
            else ByteArray(chars.size) { chars[it].code.toByte() }
        } finally { chars.fill('\u0000') }
    }
}

@Composable
internal fun AiKeyDialog(owner: AiServiceController, dismiss: () -> Unit) {
    val state by owner.state.collectAsState()
    val field = remember { AiPasswordField() }
    fun finish() { field.clearInput(); owner.cancelSettings(); dismiss() }
    DialogWindow(onCloseRequest = ::finish, title = "Image provider API key",
        state = rememberDialogState(width = 500.dp, height = 380.dp)) {
        DisposableEffect(window, field) {
            val listener = object : WindowAdapter() {
                override fun windowDeactivated(event: WindowEvent) { field.clearInput() }
            }
            window.addWindowListener(listener)
            onDispose { window.removeWindowListener(listener); field.clearInput(); owner.cancelSettings() }
        }
        MaterialTheme {
            Surface(Modifier.fillMaxSize()) {
                Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("Protected API key", style = MaterialTheme.typography.h6)
                    Text("Saved in Windows Credential Manager for this user. Saving a key does not send a request or authorize spending.")
                    SwingPanel(factory = { field }, update = { it.isEnabled = !state.busy },
                        modifier = Modifier.fillMaxWidth().height(42.dp))
                    Text(if (state.readiness?.configured == true) "A key is saved." else "No available key has been confirmed.")
                    state.failure?.let { Text(aiFailureText(it), color = MaterialTheme.colors.error) }
                    if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { owner.saveWindowsKey(field.takeBytes() ?: ByteArray(0)) }, enabled = !state.busy) { Text("Save key") }
                        TextButton(onClick = { field.clearInput(); owner.removeWindowsKey() }, enabled = !state.busy) { Text("Remove saved key") }
                        TextButton(onClick = ::finish) { Text("Close") }
                    }
                    Text("The field is cleared when this window loses focus. Input wiping is best effort; Windows and UI libraries may retain internal copies.",
                        style = MaterialTheme.typography.caption)
                }
            }
        }
    }
}
