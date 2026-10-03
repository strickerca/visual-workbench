package com.visualworkbench.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import com.visualworkbench.android.session.AndroidDiscovery
import com.visualworkbench.android.session.PairingController
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

@Composable
internal fun DiscoveryPanel(pairing: PairingController) {
    val context = LocalContext.current
    var version by remember { mutableStateOf(0) }
    var current by remember { mutableStateOf<AndroidDiscovery?>(null) }
    var failed by remember { mutableStateOf(false) }
    LaunchedEffect(version) {
        val discovery = AndroidDiscovery(context.applicationContext)
        current = discovery; failed = false
        try { discovery.start(pairing.service()); kotlinx.coroutines.awaitCancellation() }
        catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
        catch (_: Exception) { failed = true }
        finally { withContext(NonCancellable) { runCatching { discovery.close() } }; if (current === discovery) current = null }
    }
    Text("Nearby address hints", style = MaterialTheme.typography.h6)
    Text("These entries are not verified computer identities. Select the matching route and address, then choose a paired computer.", style = MaterialTheme.typography.caption)
    current?.peers.orEmpty().flatMap { it.endpoints }.distinct().take(64).forEach { address ->
        TextButton(onClick = { pairing.sessionAddress = address }, modifier = Modifier.heightIn(min = 48.dp)) { Text(address) }
    }
    current?.problem?.let { Text(it) }
    if (failed) Text("Discovery is unavailable. Scan the QR or enter an address shown by the computer.")
    OutlinedButton(onClick = { version++ }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Restart discovery") }
}
