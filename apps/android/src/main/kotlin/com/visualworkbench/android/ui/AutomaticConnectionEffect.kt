package com.visualworkbench.android.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.visualworkbench.android.editor.EditorController
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

private const val AUTOMATIC_STEP_MS = 2000L

/** Pair once, then no typing: while the app is in the foreground, connect the
 * open project to the paired computer by itself and keep that connection
 * supplied with routes. It only ever presses the same Connect the owner could
 * press; it never replaces a connection, and after the owner disconnects it
 * stays quiet until the project is opened again. */
@Composable
internal fun AutomaticConnectionEffect(editor: EditorController) {
    val context = LocalContext.current
    val owner = remember(context) { pairingActivity(context) as? LifecycleOwner }
    var foreground by remember(owner) { mutableStateOf(owner?.lifecycle?.currentState?.isAtLeast(Lifecycle.State.RESUMED) ?: true) }
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) foreground = true
            if (event == Lifecycle.Event.ON_PAUSE) foreground = false
        }
        owner?.lifecycle?.addObserver(observer)
        onDispose { owner?.lifecycle?.removeObserver(observer) }
    }
    // A project that closes and opens again between two steps is a fresh start.
    LaunchedEffect(editor) {
        snapshotFlow { editor.document == null }.collect { closed -> if (closed) editor.pairing.automaticProjectClosed() }
    }
    LaunchedEffect(editor, foreground) {
        if (!foreground) {
            try { editor.pairing.automaticRest() } catch (cancel: CancellationException) { throw cancel } catch (_: Exception) { }
            return@LaunchedEffect
        }
        while (true) {
            try {
                val connect = editor.pairing.automaticStep(
                    project = { editor.document?.let { "${it.render.revision.projectId}:${it.documentId}" } },
                    status = { editor.connection },
                    busy = { editor.busy || editor.pending != 0 },
                )
                // Switched off while a connection it had started was still on
                // its way: that connection, only waiting or looking, ends too.
                if (editor.pairing.automaticShouldStop(editor.connection)) editor.disconnect()
                if (connect) {
                    editor.connectCurrent()
                    // Connecting marks the editor busy at once; if it did not, it
                    // declined (an edit had just begun) and the next step asks again.
                    if (!editor.busy) editor.pairing.automaticMissed()
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { /* Automatic connection never interrupts the owner; the next step tries again. */ }
            delay(AUTOMATIC_STEP_MS)
        }
    }
}
