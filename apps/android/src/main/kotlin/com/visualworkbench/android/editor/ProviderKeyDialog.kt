package com.visualworkbench.android.editor

import android.text.InputFilter
import android.text.InputType
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material.AlertDialog
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Text
import androidx.compose.material.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.visualworkbench.shared.AndroidProviderKeyStore
import com.visualworkbench.shared.ProviderCredentialException
import com.visualworkbench.shared.ProviderCredentialProblem
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Root wires this explicit settings action after Application trust bootstrap.
 * Never reads the saved value into the field. Input is not remembered/saved. */
@Composable
internal fun ProviderKeyDialog(store: AndroidProviderKeyStore, lifecycle: Lifecycle, onDismiss: () -> Unit) {
    var field by remember { mutableStateOf<EditText?>(null) }
    var message by remember { mutableStateOf("Checking protected storage…") }
    var busy by remember { mutableStateOf(false) }
    var generation by remember { mutableStateOf(0L) }
    var operation by remember { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    val dismiss by rememberUpdatedState(onDismiss)
    fun clear() { field?.text?.clear(); field?.clearFocus() }
    fun describe(failure: ProviderCredentialException): String = when (failure.problem) {
        ProviderCredentialProblem.Invalid -> "Use a nonempty API key containing visible ASCII characters, up to 4096 bytes."
        ProviderCredentialProblem.Busy -> "Protected storage is busy. Try again when the previous operation has finished."
        ProviderCredentialProblem.Unavailable -> "Protected storage is unavailable. No key was shown."
    }
    LaunchedEffect(store) {
        val entered = generation
        try {
            val configured = store.isConfigured()
            if (generation == entered) message = if (configured) "A key is saved in protected storage." else "No key is saved."
        }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (failure: ProviderCredentialException) { if (generation == entered) message = describe(failure) }
    }
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) { clear(); operation?.cancel(); dismiss() }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); clear(); operation?.cancel(); field = null }
    }
    AlertDialog(
        onDismissRequest = { clear(); operation?.cancel(); onDismiss() },
        properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
        title = { Text("Image provider API key") },
        text = { Column {
            Text("Saved only in this app's protected device storage. Saving a key does not send a request or authorize spending.")
            AndroidView(modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp), factory = { context ->
                EditText(context).apply {
                    hint = "OpenAI API key"
                    inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                    isSingleLine = true
                    isSaveEnabled = false
                    importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
                    imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_ACTION_DONE
                    filters = arrayOf(InputFilter.LengthFilter(AndroidProviderKeyStore.MAX_KEY_BYTES))
                    field = this
                }
            }, update = { it.isEnabled = !busy })
            Text(message, style = MaterialTheme.typography.caption)
            Text("Input clearing is best effort: keyboard, Android and HTTP libraries may retain internal copies.", style = MaterialTheme.typography.caption)
            TextButton(enabled = !busy, onClick = {
                clear(); generation++; busy = true
                operation = scope.launch {
                    try { store.remove(); message = "Saved key removed." }
                    catch (cancelled: CancellationException) { throw cancelled }
                    catch (failure: ProviderCredentialException) { message = describe(failure) }
                    finally { busy = false }
                }
            }) { Text("Remove saved key") }
        } },
        confirmButton = { TextButton(enabled = !busy, onClick = {
            val editable = field?.text
            if (editable == null || editable.isEmpty() || editable.any { it.code !in 33..126 }) {
                clear(); message = "Enter a valid API key with visible ASCII characters."
            } else {
                val bytes = ByteArray(editable.length) { editable[it].code.toByte() }
                clear(); generation++; busy = true
                operation = scope.launch {
                    try { store.save(bytes); message = "Key saved in protected storage." }
                    catch (cancelled: CancellationException) { throw cancelled }
                    catch (failure: ProviderCredentialException) { message = describe(failure) }
                    finally { bytes.fill(0); busy = false }
                }
                // If cancellation prevents the coroutine from starting, its body
                // never gets ownership; completion still scrubs this local copy.
                operation?.invokeOnCompletion { bytes.fill(0) }
            }
        }) { Text("Save") } },
        dismissButton = { TextButton(onClick = { clear(); operation?.cancel(); onDismiss() }) { Text("Close") } },
    )
}
