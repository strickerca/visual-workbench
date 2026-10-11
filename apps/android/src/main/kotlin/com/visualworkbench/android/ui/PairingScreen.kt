package com.visualworkbench.android.ui

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Settings
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.WorkbenchScreen
import com.visualworkbench.android.session.QrScanner
import com.visualworkbench.android.session.validEndpoint
import com.visualworkbench.shared.PairedDevice
import com.visualworkbench.shared.SessionCarrier
import com.visualworkbench.shared.checkedPeerLabel

private enum class PairingMethod(val label: String) { Qr("QR code"), Code("Eight-digit code") }

@Composable
internal fun PairingScreen(editor: EditorController) {
    val state = editor.pairing
    val context = LocalContext.current
    var permission by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    var foreground by remember { mutableStateOf(true) }
    var cameraFailure by remember { mutableStateOf(false) }
    var cameraRequested by remember { mutableStateOf(false) }
    var pendingCameraEpoch by remember { mutableStateOf<Long?>(null) }
    var addingComputer by remember { mutableStateOf(false) }
    var initialChoiceMade by remember { mutableStateOf(false) }
    var completedPairing by remember { mutableStateOf(state.pairingCompletion) }
    var method by remember { mutableStateOf(PairingMethod.Qr) }
    var methodMenu by remember { mutableStateOf(false) }
    var otherOptions by remember { mutableStateOf(false) }
    var connectionDetails by remember { mutableStateOf(false) }
    var codeConnectionDetails by remember { mutableStateOf(false) }
    var renameTarget by remember { mutableStateOf<PairedDevice?>(null) }
    var renameName by remember { mutableStateOf("") }
    var revoke by remember { mutableStateOf<String?>(null) }
    val activePeers = state.activePeers()
    val available = !state.working && !editor.busy
    val changeMethod = available && state.fingerprint == null
    val selectedActivePeer = activePeers.singleOrNull { it.deviceId == state.selectedPeer }
    var peerDetails by remember(selectedActivePeer?.deviceId, selectedActivePeer?.fingerprint) { mutableStateOf(false) }
    val configured = state.configuredCarriers()
    val connectionEndpoints = runCatching { state.endpoint() }.getOrNull()
    val connectionReady = available && selectedActivePeer != null && connectionEndpoints != null && editor.pending == 0
    fun scanStillRequested(epoch: Long): Boolean = addingComputer && method == PairingMethod.Qr && foreground && state.scanner &&
        epoch == state.scanRequestEpoch && !state.working && !editor.busy
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        val epoch = pendingCameraEpoch
        pendingCameraEpoch = null
        permission = granted
        if (granted && epoch != null && epoch == state.scanRequestEpoch && foreground && addingComputer && method == PairingMethod.Qr) {
            state.scanner = true
            cameraFailure = false
        }
    }
    fun stopPairing() {
        pendingCameraEpoch = null
        state.leave()
        cameraFailure = false
        methodMenu = false
    }
    LaunchedEffect(state.peersLoaded, state.working) {
        if (!initialChoiceMade && state.peersLoaded && !state.working) {
            addingComputer = activePeers.isEmpty()
            initialChoiceMade = true
        }
    }
    LaunchedEffect(state.pairingCompletion) {
        if (state.pairingCompletion != completedPairing) {
            completedPairing = state.pairingCompletion
            addingComputer = false
            initialChoiceMade = true
            pendingCameraEpoch = null
            methodMenu = false
        }
    }
    DisposableEffect(context) {
        val activity = pairingActivity(context)
        val owner = activity as? LifecycleOwner
        val originallySecure = activity?.window?.attributes?.flags?.and(WindowManager.LayoutParams.FLAG_SECURE) != 0
        activity?.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                foreground = true
                permission = context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
            }
            if (event == Lifecycle.Event.ON_PAUSE) {
                foreground = false
                pendingCameraEpoch = null
                state.scanner = false
            }
        }
        owner?.lifecycle?.addObserver(observer)
        onDispose {
            owner?.lifecycle?.removeObserver(observer)
            if (!originallySecure) activity?.window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
            pendingCameraEpoch = null
            state.leave()
        }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = { editor.navigate(if (editor.document == null) WorkbenchScreen.Projects else WorkbenchScreen.Canvas) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("← Back") }
        Text(if (addingComputer) "Add computer" else "Connect to computer", style = MaterialTheme.typography.h4)
        if (state.working || editor.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        state.problem?.let { Text(it, color = MaterialTheme.colors.error) }
        state.namingProblem?.let { Text(it, color = MaterialTheme.colors.error) }
        if (state.namingWorking) Text("Saving computer name…", style = MaterialTheme.typography.caption)
        if (!state.peersLoaded) {
            Text(if (state.working) "Loading paired computers…" else "Paired computers could not be loaded. Refresh to try again.")
            if (!state.working) OutlinedButton(onClick = state::enter, enabled = !editor.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Refresh paired computers") }
        }
        if (addingComputer) {
            Box {
                OutlinedButton(onClick = { methodMenu = true }, enabled = changeMethod, modifier = Modifier.heightIn(min = 48.dp)) { Text("Pair using: ${method.label} ▾") }
                DropdownMenu(expanded = methodMenu, onDismissRequest = { methodMenu = false }) {
                    PairingMethod.entries.forEach { candidate ->
                        DropdownMenuItem(onClick = {
                            if (candidate != method) stopPairing()
                            method = candidate
                            methodMenu = false
                        }, enabled = changeMethod, modifier = Modifier.heightIn(min = 48.dp)) { Text(candidate.label) }
                    }
                }
            }
            Text("On the PC, open Pair devices → Pair a new phone and choose the same method. The Windows computer name identifies that screen; its network adapter is the connection to this phone. Pairing verifies the complete fingerprint on both devices.")
            when (method) {
                PairingMethod.Qr -> {
                    Text("PC: choose QR code first, choose the PC network connection, then Show QR code. Here, tap Scan QR code and frame the full square with its white border. Camera frames are not saved.")
                    if (permission && foreground && state.scanner && !cameraFailure && !state.working && !editor.busy) {
                        val scanEpoch = state.scanRequestEpoch
                        QrScanner(Modifier.fillMaxWidth().height(240.dp), { bytes ->
                            if (scanStillRequested(scanEpoch)) state.scan(bytes) else { bytes.fill(0); false }
                        }) {
                            if (scanStillRequested(scanEpoch)) { cameraFailure = true; state.scanner = false }
                        }
                        TextButton(onClick = { state.scanner = false }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Stop scanning") }
                    } else {
                        Button(onClick = {
                            stopPairing()
                            permission = context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
                            if (permission) state.scanner = true
                            else {
                                cameraRequested = true
                                pendingCameraEpoch = state.scanRequestEpoch
                                try { request.launch(Manifest.permission.CAMERA) }
                                catch (_: Exception) { pendingCameraEpoch = null; cameraFailure = true }
                            }
                        }, enabled = available && foreground, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (state.details == null) "Scan QR code" else "Scan another QR code") }
                    }
                    if (cameraFailure) Text("The camera is unavailable. Choose Eight-digit code from Pair using.")
                    else if (cameraRequested && !permission) Text("Camera access was not granted. Tap Scan QR code to retry, or choose Eight-digit code from Pair using.")
                    state.details?.let { details ->
                        Text("Computer fingerprint", style = MaterialTheme.typography.subtitle1)
                        Text(details.fingerprint.chunked(4).joinToString(" "), style = MaterialTheme.typography.caption)
                        var qrConnectionDetails by remember(details) { mutableStateOf(false) }
                        Text("Pairing uses the connection in this QR.", style = MaterialTheme.typography.caption)
                        TextButton(onClick = { qrConnectionDetails = !qrConnectionDetails }, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (qrConnectionDetails) "Hide pairing connection details" else "Pairing connection details") }
                        if (qrConnectionDetails) details.endpoints.forEach { endpoint ->
                            TextButton(onClick = { state.pairingEndpoint = endpoint }, enabled = !state.working, modifier = Modifier.heightIn(min = 48.dp)) {
                                Text((if (state.pairingEndpoint == endpoint) "✓ " else "") + endpoint)
                            }
                        }
                        Button(onClick = state::pairQr, enabled = available, modifier = Modifier.heightIn(min = 48.dp)) { Text("Add this computer") }
                    }
                }
                PairingMethod.Code -> {
                    Text("PC: choose Eight-digit code first, choose the PC network connection, then Show eight-digit code. Enter that code and pairing address here, then compare the complete fingerprint on both devices.")
                    OutlinedTextField(state.code, { if (it.length <= 8 && it.all { c -> c in '0'..'9' }) state.code = it }, label = { Text("Eight-digit code") }, singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword), visualTransformation = PasswordVisualTransformation(),
                        enabled = available, modifier = Modifier.fillMaxWidth())
                    TextButton(onClick = { codeConnectionDetails = !codeConnectionDetails }, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (codeConnectionDetails) "Hide pairing connection details" else "Pairing connection details") }
                    if (codeConnectionDetails) OutlinedTextField(state.pairingEndpoint, { if (it.length <= 128) state.pairingEndpoint = it }, label = { Text("Pairing address shown on computer") }, singleLine = true,
                        enabled = available, modifier = Modifier.fillMaxWidth())
                    if (!validEndpoint(state.pairingEndpoint)) Text("Open Pairing connection details and enter the address and port shown on the computer.", style = MaterialTheme.typography.caption)
                    Button(onClick = state::pairCode, enabled = available && state.code.length == 8 && validEndpoint(state.pairingEndpoint), modifier = Modifier.heightIn(min = 48.dp)) { Text("Check code") }
                }
            }
            if (activePeers.isNotEmpty()) TextButton(onClick = { stopPairing(); addingComputer = false; initialChoiceMade = true }, enabled = changeMethod, modifier = Modifier.heightIn(min = 48.dp)) { Text("Use a paired computer") }
        } else if (state.peersLoaded) {
            Text("Choose a paired computer", style = MaterialTheme.typography.h6)
            Text("Already paired? Select its saved name and receive the project below. A new QR or code is not required.", style = MaterialTheme.typography.caption)
            if (activePeers.isEmpty()) Text("No active paired computers.")
            activePeers.forEach { peer ->
                TextButton(onClick = { state.choosePeer(peer.deviceId) }, enabled = available, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                    Text((if (state.selectedPeer == peer.deviceId) "✓ " else "") + state.peerDisplayName(peer))
                }
            }
            selectedActivePeer?.let { peer ->
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { renameTarget = peer; renameName = state.peerName(peer) }, enabled = available && !state.namingWorking && state.namesWritable, modifier = Modifier.heightIn(min = 48.dp)) { Text("Rename") }
                    TextButton(onClick = { peerDetails = !peerDetails }, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (peerDetails) "Hide computer details" else "Computer details") }
                }
                if (peerDetails) {
                    Text("Name is saved only on this phone. Pairing uses this computer's verified identity.", style = MaterialTheme.typography.caption)
                    Text("Computer fingerprint", style = MaterialTheme.typography.subtitle1)
                    Text(peer.fingerprint.chunked(4).joinToString(" "), style = MaterialTheme.typography.caption)
                }
            }
            OutlinedButton(onClick = { stopPairing(); addingComputer = true; initialChoiceMade = true }, enabled = changeMethod, modifier = Modifier.heightIn(min = 48.dp)) { Text("Add computer") }
            Divider()
            Text(sessionRouteLabel(state.carrier), style = MaterialTheme.typography.h6)
            Text(if (state.carrier == SessionCarrier.QuicTether) "Enable USB tethering on this phone. PC: Pair devices → choose this paired phone → open or import a Workbench project if needed → choose the PC tether network adapter → Use selected connection for USB project → Connect phone to this project."
                else "On the PC, start its Workbench project connection using ${sessionRouteName(state.carrier)} in Other connection options. Remote Paint requires USB tethering.")
            Text("The Workbench project folder (.vwb) supplies the connection. The PC selects the Paint document/window separately.", style = MaterialTheme.typography.caption)
            TextButton(onClick = {
                try { context.startActivity(Intent("android.settings.TETHER_SETTINGS")) }
                catch (_: Exception) { try { context.startActivity(Intent(Settings.ACTION_WIRELESS_SETTINGS)) } catch (_: Exception) { editor.transferMessage("Open Android Settings and choose USB tethering.") } }
            }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Open USB tethering settings") }
            if (state.canUsePairingConnection()) {
                OutlinedButton(onClick = state::usePairingConnection, enabled = available, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Use pairing connection for ${sessionRouteName(state.carrier)}") }
                Text("Copies only this route's address. Start the project on the same PC connection; pairing alone does not start a project or establish USB.", style = MaterialTheme.typography.caption)
            }
            TextButton(onClick = { connectionDetails = !connectionDetails }, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (connectionDetails) "Hide connection details" else "Connection details") }
            if (connectionDetails) {
                OutlinedTextField(state.sessionAddress, { if (it.length <= 128) state.sessionAddress = it }, label = { Text(sessionAddressLabel(state.carrier)) }, singleLine = true,
                    enabled = available, modifier = Modifier.fillMaxWidth())
                Text("Use the project session address shown on the PC. It may differ from the pairing connection.", style = MaterialTheme.typography.caption)
                Text("No project open on the PC? Its paired-phone screen offers Open project to choose project.sqlite inside a saved .vwb folder, or Import file to create one from an image. That connection screen returns after the picker. Choose the PC network adapter and Use selected connection for USB project, then Connect phone to this project. The File menu also opens or imports projects.", style = MaterialTheme.typography.caption)
            }
            if (configured.isNotEmpty()) Text("Configured connection order: ${configured.joinToString(" → ") { sessionRouteName(it) }}.", style = MaterialTheme.typography.caption)
            when {
                selectedActivePeer == null -> Text("Choose a paired computer above.", style = MaterialTheme.typography.caption)
                configured.isEmpty() -> Text("Use the pairing connection above, or open Connection details to enter the PC project session address.", style = MaterialTheme.typography.caption)
                connectionEndpoints == null -> Text("Open Connection details and check each configured route's address and port. Leave unavailable routes empty.", style = MaterialTheme.typography.caption)
                editor.pending != 0 -> Text("Finish the current edits before connecting.", style = MaterialTheme.typography.caption)
            }
            Button(onClick = editor::receiveProject, enabled = connectionReady,
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Receive computer project") }
            Text("Receive opens the canvas and starts connecting its copy. Tap More → Connect to return here, wait for USB, then Remote Paint. If this same computer-owned project is already open, use Connect current project.", style = MaterialTheme.typography.caption)
            if (editor.document != null) {
                OutlinedButton(onClick = editor::connectCurrent, enabled = connectionReady,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) { Text("Connect current project") }
                Text("A computer-owned project connects to its host. A phone-owned project listens at a phone address configured in Other connection options; ownership never changes implicitly.", style = MaterialTheme.typography.caption)
            }
            editor.connection?.let { status ->
                Text(editor.connectionLabel())
                Button(onClick = editor::openRemoteComputer, enabled = status.carrier == SessionCarrier.QuicTether && !editor.busy, modifier = Modifier.heightIn(min = 48.dp)) { Text("Remote Paint") }
                Text("PC: open your Paint document, then Workbench → Remote Krita / Paint… → select the exact Paint window → Grant selected window. Phone: Remote Paint → Request PC control when available. Control requires the USB link.", style = MaterialTheme.typography.caption)
                TextButton(onClick = editor::disconnect, modifier = Modifier.heightIn(min = 48.dp)) { Text("Disconnect") }
            }
            OutlinedButton(onClick = { otherOptions = !otherOptions }, modifier = Modifier.heightIn(min = 48.dp)) { Text(if (otherOptions) "Hide other connection options" else "Other connection options") }
            if (otherOptions) {
                Text("Connection route", style = MaterialTheme.typography.h6)
                listOf(SessionCarrier.QuicTether, SessionCarrier.QuicWifi, SessionCarrier.TcpAdb).forEach { carrier ->
                    Row {
                        RadioButton(state.carrier == carrier, onClick = { state.carrier = carrier }, enabled = available, modifier = Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp))
                        TextButton(onClick = { state.carrier = carrier }, enabled = available, modifier = Modifier.heightIn(min = 48.dp)) { Text(sessionRouteLabel(carrier)) }
                    }
                }
                Text("Each route keeps its own address. All configured routes reconnect in USB tethering, Wi-Fi, then ADB order. Leave unavailable routes empty; no address or trust is inferred from another route.", style = MaterialTheme.typography.caption)
                if (editor.document != null) state.localAddresses.forEach { candidate ->
                    TextButton(onClick = {
                        val port = state.sessionAddress.substringAfterLast(':', "45450").toIntOrNull()?.takeIf { it in 1..65535 } ?: 45450
                        val host = if (':' in candidate.address) "[${candidate.address}]" else candidate.address
                        state.sessionAddress = "$host:$port"
                    }, enabled = available, modifier = Modifier.heightIn(min = 48.dp)) { Text("Listen on phone address ${candidate.address}") }
                }
                if (foreground) DiscoveryPanel(state)
                OutlinedButton(onClick = state::enter, enabled = available, modifier = Modifier.heightIn(min = 48.dp)) { Text("Refresh paired computers") }
                activePeers.forEach { peer ->
                    TextButton(onClick = { revoke = peer.deviceId }, enabled = available, modifier = Modifier.heightIn(min = 48.dp)) { Text("Revoke ${state.peerDisplayName(peer)}") }
                }
                editor.connection?.let { status ->
                    if (status.echoSamples > 0u) Text("Echo RTT p50 ${status.echoRttP50Ms?.let { "%.1f".format(it) } ?: "—"} ms · p95 ${status.echoRttP95Ms?.let { "%.1f".format(it) } ?: "—"} ms (${status.echoSamples} samples)", style = MaterialTheme.typography.caption)
                }
            }
        }
    }
    renameTarget?.let { peer ->
        val current = activePeers.singleOrNull { it.deviceId == peer.deviceId && it.fingerprint == peer.fingerprint }
        AlertDialog(onDismissRequest = { renameTarget = null }, title = { Text("Rename computer") }, text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("This name is saved only on this phone. It does not change pairing or the computer's identity.")
                OutlinedTextField(renameName, { if (it.length <= 256) renameName = it }, label = { Text("Computer name") }, singleLine = true)
                if (current == null || state.selectedPeer != peer.deviceId) Text("This computer changed. Close and choose it again.", color = MaterialTheme.colors.error)
            }
        }, confirmButton = { TextButton(onClick = { state.renamePeer(peer.deviceId, peer.fingerprint, renameName); renameTarget = null },
            enabled = available && !state.namingWorking && state.namesWritable && current != null && state.selectedPeer == peer.deviceId && checkedPeerLabel(renameName) != null,
            modifier = Modifier.heightIn(min = 48.dp)) { Text("Save name") } },
            dismissButton = { TextButton(onClick = { renameTarget = null }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Cancel") } })
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

private fun sessionRouteName(carrier: SessionCarrier): String = when (carrier) {
    SessionCarrier.QuicTether -> "USB tethering"
    SessionCarrier.QuicWifi -> "Wi-Fi"
    SessionCarrier.TcpAdb -> "ADB tunnel"
}

private fun sessionRouteLabel(carrier: SessionCarrier): String = when (carrier) {
    SessionCarrier.QuicTether -> "USB tethering (recommended)"
    SessionCarrier.QuicWifi -> "Wi-Fi"
    SessionCarrier.TcpAdb -> "ADB tunnel (developer option)"
}

private fun sessionAddressLabel(carrier: SessionCarrier): String = when (carrier) {
    SessionCarrier.QuicTether -> "USB session address shown on computer"
    SessionCarrier.QuicWifi -> "Wi-Fi session address"
    SessionCarrier.TcpAdb -> "ADB session address"
}

private fun pairingActivity(context: Context): Activity? {
    var current = context
    repeat(16) {
        if (current is Activity) return current as Activity
        current = (current as? ContextWrapper)?.baseContext ?: return null
    }
    return null
}
