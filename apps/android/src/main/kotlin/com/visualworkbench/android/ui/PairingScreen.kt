package com.visualworkbench.android.ui

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.provider.Settings
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.android.session.QrScanner
import com.visualworkbench.shared.SessionCarrier

@Composable
internal fun PairingScreen(editor: EditorController) {
    val state = editor.pairing
    val context = LocalContext.current
    var permission by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    var foreground by remember { mutableStateOf(true) }
    var cameraFailure by remember { mutableStateOf(false) }
    var revoke by remember { mutableStateOf<String?>(null) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        permission = it; if (it) { state.scanner = true; cameraFailure = false }
    }
    LaunchedEffect(Unit) { if (!permission) request.launch(Manifest.permission.CAMERA) }
    DisposableEffect(context) {
        val activity = pairingActivity(context)
        val owner = activity as? LifecycleOwner
        val originallySecure = activity?.window?.attributes?.flags?.and(WindowManager.LayoutParams.FLAG_SECURE) != 0
        activity?.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) foreground = true
            if (event == Lifecycle.Event.ON_PAUSE) foreground = false
        }
        owner?.lifecycle?.addObserver(observer)
        onDispose {
            owner?.lifecycle?.removeObserver(observer)
            if (!originallySecure) activity?.window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
            state.leave()
        }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = { editor.navigate(if (editor.document == null) WorkbenchScreen.Projects else WorkbenchScreen.Canvas) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("← Back") }
        Text("Pair a computer", style = MaterialTheme.typography.h4)
        Text("Pairing stays on your local network. The phone verifies the computer's certificate; discovery is only an address hint.")
        if (state.working || editor.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (permission && foreground && state.scanner && !cameraFailure && !state.working) {
            QrScanner(Modifier.fillMaxWidth().height(240.dp), state::scan) { cameraFailure = true }
            Text("Point the camera at the pairing QR on the computer. Frames are not saved.", style = MaterialTheme.typography.caption)
        } else if (!permission || cameraFailure) {
            Text(if (cameraFailure) "The camera is unavailable. You can pair with an eight-digit code." else "Camera access was not granted. You can retry permission or use the code below.")
            TextButton(onClick = { cameraFailure = false; request.launch(Manifest.permission.CAMERA) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Allow camera") }
        } else TextButton(onClick = { state.scanner = true }, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) { Text("Scan another QR") }
        state.details?.let { details ->
            Text("Computer fingerprint", style = MaterialTheme.typography.subtitle1)
            Text(details.fingerprint.chunked(4).joinToString(" "), style = MaterialTheme.typography.caption)
            details.endpoints.forEach { endpoint ->
                TextButton(onClick = { state.pairingEndpoint = endpoint }, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text((if (state.pairingEndpoint == endpoint) "✓ " else "") + endpoint)
                }
            }
            Button(onClick = state::pairQr, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) { Text("Pair with this computer") }
        }
        Divider()
        Text("Pair with a code", style = MaterialTheme.typography.h6)
        OutlinedTextField(state.pairingEndpoint, { if (it.length <= 128) state.pairingEndpoint = it }, label = { Text("Pairing address shown on computer") }, singleLine = true,
            enabled = !state.working, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(state.code, { if (it.length <= 8 && it.all { c -> c in '0'..'9' }) state.code = it }, label = { Text("Eight-digit code") }, singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword), visualTransformation = PasswordVisualTransformation(),
            enabled = !state.working, modifier = Modifier.fillMaxWidth())
        Button(onClick = state::pairCode, enabled = !state.working && state.code.length == 8, modifier = Modifier.heightIn(min = 48.dp)) { Text("Check code") }
        state.problem?.let { Text(it, color = MaterialTheme.colors.error) }
        Divider()
        Text("Paired computers", style = MaterialTheme.typography.h6)
        if (state.peers.none { it.revokedAtMs == null }) Text("No active paired computers.")
        state.peers.filter { it.revokedAtMs == null }.forEachIndexed { index, peer ->
            Row(Modifier.fillMaxWidth()) {
                TextButton(onClick = { state.choosePeer(peer.deviceId) }, modifier = Modifier.weight(1f).heightIn(min = 48.dp)) {
                    Text((if (state.selectedPeer == peer.deviceId) "✓ " else "") + "Computer ${index + 1} · ${peer.fingerprint.take(12)}")
                }
                TextButton(onClick = { revoke = peer.deviceId }, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) { Text("Revoke") }
            }
        }
        OutlinedButton(onClick = state::enter, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) { Text("Refresh paired computers") }
        if (foreground) DiscoveryPanel(state)
        Divider()
        Text("Open a shared project", style = MaterialTheme.typography.h6)
        Text("Choose the session address shown by the computer. A phone-created host project instead listens at this phone's local address; host ownership is never changed implicitly.")
        if (editor.document != null) state.localAddresses.forEach { candidate ->
            TextButton(onClick = {
                val port = state.sessionAddress.substringAfterLast(':', "45450").toIntOrNull()?.takeIf { it in 1..65535 } ?: 45450
                val host = if (':' in candidate.address) "[${candidate.address}]" else candidate.address
                state.sessionAddress = "$host:$port"
            }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Listen on phone address ${candidate.address}") }
        }
        OutlinedTextField(state.sessionAddress, { if (it.length <= 128) state.sessionAddress = it }, label = { Text("Session address and port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        listOf(SessionCarrier.QuicTether to "USB tethering", SessionCarrier.QuicWifi to "Wi-Fi", SessionCarrier.TcpAdb to "ADB tunnel (developer option)").forEach { (carrier, label) ->
            Row {
                RadioButton(state.carrier == carrier, onClick = { state.carrier = carrier }, modifier = Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp))
                TextButton(onClick = { state.carrier = carrier }, modifier = Modifier.heightIn(min = 48.dp)) { Text(label) }
            }
        }
        Text("Enter each route's actual address separately. Configured routes reconnect in USB tethering, Wi-Fi, then ADB order. Leave unavailable routes empty; no address or trust is inferred from another route.", style = MaterialTheme.typography.caption)
        Button(onClick = editor::receiveProject, enabled = !editor.busy && !state.working && state.selectedPeer != null && editor.pending == 0,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Receive computer project") }
        OutlinedButton(onClick = editor::connectCurrent, enabled = editor.document != null && !editor.busy && !state.working && editor.pending == 0,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Connect current project") }
        editor.connection?.let { status ->
            Text(editor.connectionLabel())
            Button(onClick=editor::openRemoteComputer,enabled=status.carrier==SessionCarrier.QuicTether&&!editor.busy,modifier=Modifier.heightIn(min=48.dp)){Text("Remote Krita / Paint")}
            Text("Remote input uses the selected USB tether route and a separate grant on the computer.",style=MaterialTheme.typography.caption)
            if (status.echoSamples > 0u) Text("Echo RTT p50 ${status.echoRttP50Ms?.let { "%.1f".format(it) } ?: "—"} ms · p95 ${status.echoRttP95Ms?.let { "%.1f".format(it) } ?: "—"} ms (${status.echoSamples} samples)", style = MaterialTheme.typography.caption)
            TextButton(onClick = editor::disconnect, modifier = Modifier.heightIn(min = 48.dp)) { Text("Disconnect") }
        }
        TextButton(onClick = {
            try { context.startActivity(Intent("android.settings.TETHER_SETTINGS")) }
            catch (_: Exception) { try { context.startActivity(Intent(Settings.ACTION_WIRELESS_SETTINGS)) } catch (_: Exception) { editor.transferMessage("Open Android Settings and choose USB tethering.") } }
        }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Open USB tethering settings") }
    }
    state.fingerprint?.let { fingerprint ->
        AlertDialog(onDismissRequest = state::decline, title = { Text("Compare the fingerprint") }, text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Compare this complete computer fingerprint with the one displayed on the computer. Confirm only if both match.")
                Text(fingerprint.chunked(4).joinToString(" "))
            }
        }, confirmButton = { TextButton(onClick = state::confirmFingerprint, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) { Text("Fingerprints match") } },
            dismissButton = { TextButton(onClick = state::decline, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) { Text("Decline") } })
    }
    revoke?.let { peer -> AlertDialog(onDismissRequest = { revoke = null }, title = { Text("Revoke this computer?") }, text = { Text("Active sessions close and this computer must pair again. Projects saved on this phone remain intact.") },
        confirmButton = { TextButton(onClick = { revoke = null; editor.revoke(peer) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Revoke pairing") } },
        dismissButton = { TextButton(onClick = { revoke = null }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Keep paired") } }) }
}

private fun pairingActivity(context: Context): Activity? {
    var current = context
    repeat(16) {
        if (current is Activity) return current as Activity
        current = (current as? ContextWrapper)?.baseContext ?: return null
    }
    return null
}
