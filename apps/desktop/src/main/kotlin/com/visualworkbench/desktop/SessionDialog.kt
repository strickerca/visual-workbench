package com.visualworkbench.desktop

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindow
import androidx.compose.ui.window.rememberDialogState
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.common.BitMatrix
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

@Composable
internal fun SessionDialog(sessions: DesktopSessionController, editor: EditorController, assistance: ConnectionAssistanceController,
    draft: ConnectionDraft, changeDraft: ((ConnectionDraft) -> ConnectionDraft) -> Unit, importFile: () -> Unit, openProject: () -> Unit, dismiss: () -> Unit) {
    val connection by sessions.state.collectAsState()
    val document by editor.state.collectAsState()
    val assistanceState by assistance.state.collectAsState()
    var addresses by remember { mutableStateOf<List<LocalSessionAddress>>(emptyList()) }
    var addressError by remember { mutableStateOf<String?>(null) }
    var addressRevision by remember { mutableStateOf(0) }
    var refreshingAddresses by remember { mutableStateOf(true) }
    var connectionDetails by remember { mutableStateOf(false) }
    var computerDetails by remember { mutableStateOf(false) }
    var requestedNewPhone by remember { mutableStateOf<Boolean?>(null) }
    val thisComputer = remember { computerLabel(try { System.getenv("COMPUTERNAME") } catch (_: SecurityException) { null }) }
    val activePeers = connection.paired.filter { it.revokedAtMs == null }
    val screenPath = connectionScreenPath(connection.ready, connection.paired, requestedNewPhone, connection.pairing)
    val bind = draft.bind
    val method = draft.method
    var methodMenu by remember { mutableStateOf(false) }
    val peerEndpoint = draft.peerEndpoint
    var code by remember { mutableStateOf("") }
    var renaming by remember { mutableStateOf<PairedDevice?>(null) }
    var nameDraft by remember { mutableStateOf("") }
    val local = draft.local
    val remote = draft.remote
    val expectedProject = draft.expectedProject
    var otherOptions by remember { mutableStateOf(false) }
    val phonePort = draft.phonePort
    val hostPort = draft.hostPort
    var compared by remember(connection.fingerprint) { mutableStateOf(false) }
    var now by remember { mutableStateOf(System.currentTimeMillis()) }
    val chosenAddress = draft.selectedConnection(addresses, refreshingAddresses)
    val chosenPeer = draft.selectedPeer(connection.paired)
    var peerDetails by remember(chosenPeer?.deviceId, chosenPeer?.fingerprint) { mutableStateOf(false) }
    val renameTarget = renaming?.let { target -> connection.paired.firstOrNull {
        it.deviceId == target.deviceId && it.fingerprint == target.fingerprint && it.revokedAtMs == null } }
    val projectEndpoints = when (document.sessionRole?.isHost) { true -> local.endpoints(); false -> remote.endpoints(); null -> emptyList() }
    val projectBlocker = draft.projectBlocker(connection.ready, connection.pairing, document.busy,
        document.document != null, document.sessionRole, connection.paired, addresses, refreshingAddresses)
    val usbBlocker = draft.projectBlocker(connection.ready, connection.pairing, document.busy,
        document.document != null, document.sessionRole, connection.paired, addresses, refreshingAddresses, configureUsb = true)
    val receiveBlocker = draft.receiveBlocker(connection.ready, connection.pairing, document.busy,
        connection.paired, addresses, refreshingAddresses)
    val pairingBlocker = draft.pairingBlocker(connection.ready, connection.pairing, refreshingAddresses, code)
    LaunchedEffect(Unit) {
        sessions.refreshTrust()
        // Attachment refreshes its own role. Reopening after a picker must not
        // enqueue an inspection against the project that the picker replaces.
    }
    LaunchedEffect(addressRevision) {
        refreshingAddresses = true
        addressError = null
        try {
            val fresh = localSessionAddresses()
            addresses = fresh
            changeDraft { it.reconcileConnections(fresh) }
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) {
            addresses = emptyList()
            changeDraft { it.reconcileConnections(emptyList()) }
            addressError = "Local addresses could not be refreshed. Retry Refresh, or enter a known numeric pairing address in Other connection options."
        } finally { refreshingAddresses = false }
    }
    LaunchedEffect(connection.ready, connection.paired) { if (connection.ready) changeDraft { it.reconcilePeers(connection.paired) } }
    LaunchedEffect(connection.pairing) { if (connection.pairing) methodMenu = false }
    LaunchedEffect(chosenPeer?.deviceId, chosenPeer?.fingerprint) { renaming = null; nameDraft = "" }
    LaunchedEffect(connection.display) { while (connection.display != null) { now = System.currentTimeMillis(); delay(1000) } }
    DisposableEffect(Unit) { onDispose { code = ""; sessions.cancelPairing(); sessions.stopDiscovery() } }
    DialogWindow(onCloseRequest = dismiss, title = "Connect a phone", state = rememberDialogState(width = 760.dp, height = 800.dp)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Connect a phone", style = MaterialTheme.typography.h5)
                Text("This Windows computer: $thisComputer", style = MaterialTheme.typography.subtitle1)
                Text("The PC name identifies this screen. Choose phones by their saved names; PC network adapters are connection options.", style = MaterialTheme.typography.caption)
                when (screenPath) {
                    ConnectionScreenPath.Loading -> {
                        Text(if (connection.message == null) "Opening protected pairing storage and saved phones…" else "This computer's protected pairing storage is unavailable. Review its local startup status before retrying.", color = MaterialTheme.colors.secondary)
                        connection.message?.let { Text("Local startup status: $it", style = MaterialTheme.typography.caption) }
                    }
                    ConnectionScreenPath.PairNewPhone -> {
                        Text("Pair a new phone", style = MaterialTheme.typography.h6)
                        Text("Choose how to pair here, then use the same method on the phone's Add computer screen.", style = MaterialTheme.typography.body2)
                        Box {
                            OutlinedButton(onClick = { methodMenu = true }, enabled = !connection.pairing) { Text("Pair using: ${method.label} ▾") }
                            DropdownMenu(expanded = methodMenu && !connection.pairing, onDismissRequest = { methodMenu = false }) {
                                for (option in PairingMethod.entries) DropdownMenuItem(onClick = { changeDraft { it.copy(method = option) }; methodMenu = false }) { Text(option.label) }
                            }
                        }
                        when (method) {
                            PairingMethod.Qr, PairingMethod.Code -> {
                                Text(if (method == PairingMethod.Qr) "Show this computer's QR here, then choose Scan QR code on the phone." else "Show this computer's code here, then enter it on the phone.", style = MaterialTheme.typography.body2)
                                PcConnectionPicker(addresses, chosenAddress, refreshingAddresses, !connection.pairing,
                                    refresh = { addressRevision++ }, choose = { address -> changeDraft { it.selectConnection(address) } })
                                addressError?.let { Text(it, color = MaterialTheme.colors.secondary) }
                                PairingConnectionDetails(chosenAddress, bind, connectionDetails) { connectionDetails = !connectionDetails }
                                Button(onClick = { sessions.offer(bind, method == PairingMethod.Code) }, enabled = pairingBlocker == null) {
                                    Text(if (connection.pairing && connection.display == null && connection.fingerprint == null) "Preparing pairing…" else if (method == PairingMethod.Qr) "Show QR code" else "Show eight-digit code")
                                }
                                pairingBlocker?.let { Text(pairingBlockerMessage(it), color = MaterialTheme.colors.secondary, style = MaterialTheme.typography.caption) }
                                if (connection.pairing && connection.display == null && connection.fingerprint == null) LinearProgressIndicator(Modifier.fillMaxWidth())
                                connection.message?.let { Text("Pairing status: $it", color = MaterialTheme.colors.secondary) }
                                connection.display?.let { display ->
                                    val remaining = ((display.offer.expiresAtMs.toLong() - now).coerceAtLeast(0) + 999) / 1000
                                    Text(if (remaining > 0) "Pairing offer active · $remaining seconds remaining. ${if (method == PairingMethod.Qr) "Scan the full QR on the phone." else "Enter this code and pairing address on the phone."}" else "This offer has expired. Cancel pairing, then create a new offer.")
                                    if (method == PairingMethod.Qr) {
                                        val qrText = display.qrText
                                        if (qrText == null) Text("QR data is unavailable. Cancel pairing, then choose Eight-digit code.", color = MaterialTheme.colors.secondary)
                                        else QrCode(qrText)
                                    }
                                    if (method == PairingMethod.Code) {
                                        if (display.offer.code.isNotEmpty()) Text(display.offer.code, style = MaterialTheme.typography.h4)
                                        else Text("The code is unavailable. Cancel pairing, then create a new offer.", color = MaterialTheme.colors.secondary)
                                    }
                                    if (method == PairingMethod.Code || connectionDetails) SelectionContainer { Text("Pairing address: ${display.offer.endpoint}", style = MaterialTheme.typography.caption) }
                                }
                            }
                            PairingMethod.Join -> {
                                Text("Use the eight-digit code and pairing address displayed by the other device.", style = MaterialTheme.typography.body2)
                                OutlinedTextField(peerEndpoint, { value -> changeDraft { it.copy(peerEndpoint = value.take(128)) } }, label = { Text("Other device pairing address:port") }, singleLine = true, enabled = !connection.pairing, modifier = Modifier.fillMaxWidth())
                                OutlinedTextField(code, { code = it.filter(Char::isDigit).take(8) }, label = { Text("Eight-digit code") }, singleLine = true, enabled = !connection.pairing)
                                Button(onClick = { val entered = code; code = ""; sessions.joinCode(entered, peerEndpoint) }, enabled = pairingBlocker == null) { Text(if (connection.pairing && connection.fingerprint == null) "Joining…" else "Join and compare fingerprints") }
                                pairingBlocker?.let { Text(pairingBlockerMessage(it), style = MaterialTheme.typography.caption) }
                                if (connection.pairing && connection.fingerprint == null) LinearProgressIndicator(Modifier.fillMaxWidth())
                                connection.message?.let { Text("Pairing status: $it", color = MaterialTheme.colors.secondary) }
                            }
                        }
                        if (connection.pairing) TextButton(onClick = sessions::cancelPairing) { Text("Cancel pairing") }
                        connection.fingerprint?.let { fingerprint ->
                            Text("Compare this complete fingerprint on both devices before confirming.", style = MaterialTheme.typography.subtitle1)
                            SelectionContainer { Text(fingerprint.chunked(8).joinToString(" ")) }
                            Row { Checkbox(compared, { compared = it }, enabled = !connection.confirming); Text("The complete fingerprints match on both devices.", Modifier.padding(top = 12.dp)) }
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Button(onClick = { sessions.confirmFingerprint(fingerprint, compared) }, enabled = compared && !connection.confirming) { Text("Confirm pairing") }
                                TextButton(onClick = sessions::decline, enabled = !connection.confirming) { Text("Decline") }
                            }
                        }
                        if (activePeers.isNotEmpty()) TextButton(onClick = { requestedNewPhone = false }, enabled = !connection.pairing) { Text("Use a paired phone instead") }
                    }
                    ConnectionScreenPath.PairedProject -> {
                        Text("Choose a paired phone", style = MaterialTheme.typography.h6)
                        Text(if (activePeers.isEmpty()) "No currently paired phones are available. Choose Pair a new phone to add one." else "Already paired: select the phone by its saved name, then connect the project below. You do not need another QR or code.", style = MaterialTheme.typography.caption)
                        TextButton(onClick = { requestedNewPhone = true }, enabled = !connection.pairing) { Text("Pair a new phone…") }
                        for (device in connection.paired.filter { it.revokedAtMs == null }) {
                            Row {
                                RadioButton(chosenPeer?.let { it.deviceId == device.deviceId && it.fingerprint == device.fingerprint } == true,
                                    onClick = { changeDraft { it.selectPeer(device) } })
                                Text(phoneName(device, connection), Modifier.padding(top = 12.dp))
                            }
                        }
                        chosenPeer?.let { device ->
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                TextButton(onClick = { renaming = device; nameDraft = connection.labels?.nameFor(device.deviceId, device.fingerprint) ?: phoneName(device, connection) }, enabled = connection.labels?.writable == true && !connection.labelSaving && !connection.pairing) { Text("Rename phone") }
                                TextButton(onClick = { peerDetails = !peerDetails }) { Text(if (peerDetails) "Hide device details" else "Device details ▾") }
                            }
                            if (peerDetails) SelectionContainer { Column {
                                Text("Local label only. Pairing and project operations use the saved identity, not this name.", style = MaterialTheme.typography.caption)
                                Text("Device ID: ${device.deviceId}\nFull fingerprint: ${device.fingerprint}", style = MaterialTheme.typography.caption)
                            } }
                        }
                        connection.labels?.issue?.let { issue ->
                            Text(when (issue) {
                                PeerLabelIssue.InvalidName -> "Use a single line of 1–64 characters without control or direction-formatting characters."
                                PeerLabelIssue.InvalidPeer -> "The selected phone identity changed. Select it again before renaming."
                                else -> "Local names could not be saved or read. Pairing and saved trust are unchanged; available names or defaults remain usable. No trust or identity files were changed."
                            }, style = MaterialTheme.typography.caption, color = MaterialTheme.colors.secondary)
                        }
                        if (renaming != null) {
                            Text("Name this phone on this PC", style = MaterialTheme.typography.subtitle1)
                            OutlinedTextField(nameDraft, { nameDraft = it.take(256) }, label = { Text("Local device name") }, enabled = !connection.labelSaving, singleLine = true, modifier = Modifier.fillMaxWidth())
                            Text("Up to 64 characters. The name is local and is never a network address or proof of identity.", style = MaterialTheme.typography.caption)
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Button(onClick = { renameTarget?.let { sessions.renamePeer(it.deviceId, it.fingerprint, nameDraft) }; renaming = null; nameDraft = "" }, enabled = renameTarget != null && connection.labels?.writable == true && !connection.labelSaving && !connection.pairing && checkedPeerLabel(nameDraft) != null) { Text("Save name") }
                                TextButton(onClick = { renaming = null; nameDraft = "" }) { Text("Cancel") }
                            }
                            if (renameTarget == null) Text("That paired phone is no longer available. Its name was not changed.", color = MaterialTheme.colors.secondary)
                        }
                        for (issue in draft.issues) Text(when (issue) {
                            ConnectionDraftIssue.ConnectionUnavailable -> "The selected PC connection is no longer available. Choose its current PC network connection below. Manual addresses were kept."
                            ConnectionDraftIssue.CopiedUsbRetired -> "The connection used for the USB project changed. Choose Use selected connection for USB project again, or review a manual listener in Other connection options."
                            ConnectionDraftIssue.PeerUnavailable -> "The selected phone is unavailable, revoked, or has a different fingerprint. Select a currently paired phone again."
                        }, color = MaterialTheme.colors.secondary)
                        Divider()
                        Text("Connect the Workbench project", style = MaterialTheme.typography.h6)
                        Text("The Workbench project folder (.vwb) supplies the connection. Paint's document/window is selected separately on the PC; receiving the project does not select Paint.", style = MaterialTheme.typography.body2)
                        Text(document.document?.title?.let { "Current Workbench document: $it" } ?: "No Workbench project open")
                        if (document.document == null) {
                            Text(if (connection.ready) "Use File → Import file to import an image into a Workbench project, or File → Open project to choose project.sqlite inside a saved .vwb folder." else if (connection.message == null) "Wait for local pairing storage to finish opening before setting up this project connection." else "Resolve the local pairing identity or storage problem above before opening or importing a project for this connection.", style = MaterialTheme.typography.body2)
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Button(onClick = importFile, enabled = connection.ready && !document.busy && !connection.pairing) { Text("Import file…") }
                                OutlinedButton(onClick = openProject, enabled = connection.ready && !document.busy && !connection.pairing) { Text("Open project…") }
                            }
                            if (connection.ready) Text("The connection dialog returns after the picker closes. Your selected connection and phone stay selected while they remain available. Opening a project does not start sharing it.", style = MaterialTheme.typography.caption)
                        }
                        Text(document.sessionRole?.let { if (it.isHost) "This PC hosts the Workbench project. Use the selected connection for USB project, choose Connect phone to this project, then Receive computer project on the phone." else "Local replica · ${it.pending} pending · ${it.blocked} blocked. Connect to its original host; project ownership stays unchanged." } ?: if (document.document == null) { if (connection.ready) "Open a project or import an image first. Receive from another device is in Other connection options." else "Local pairing startup must be ready before setting up this connection." } else "Project ownership could not be verified yet. Check the current project status below, then retry the role check.", style = MaterialTheme.typography.body2)
                        PcConnectionPicker(addresses, chosenAddress, refreshingAddresses, !connection.pairing,
                            refresh = { addressRevision++ }, choose = { address -> changeDraft { it.selectConnection(address) } })
                        addressError?.let { Text(it, color = MaterialTheme.colors.secondary) }
                        PairingConnectionDetails(chosenAddress, bind, connectionDetails) { connectionDetails = !connectionDetails }
                        if (document.document != null) Text("USB tethering (recommended for Paint)", style = MaterialTheme.typography.subtitle1)
                        if (document.sessionRole?.isHost == true) {
                            Text(if (local.tether.isBlank()) "Choose Use selected connection to prepare the USB project connection, or enter a listener address in Other connection options." else "USB project connection selected. Choose Connect phone to this project to start sharing it.", style = MaterialTheme.typography.caption)
                            TextButton(onClick = { changeDraft { it.useSelectedUsb(addresses, refreshingAddresses) } }, enabled = usbBlocker == null) { Text(if (local.tether.isBlank()) "Use selected connection for USB project" else "Replace USB listener with selected connection") }
                            if (usbBlocker != null && usbBlocker != projectBlocker) Text(projectBlockerMessage(usbBlocker, true), style = MaterialTheme.typography.caption)
                            Button(onClick = { chosenPeer?.let { editor.connectSession(sessions, it.deviceId, local.endpoints(), true, chosenAddress?.interfaceIndex) } }, enabled = projectBlocker == null) { Text("Connect phone to this project") }
                            Text("On the phone, select this computer and use the pairing connection for USB when offered, then Receive computer project. If the same computer-owned project is already open, choose Connect current project. Manual addresses remain in Other connection options. Copying a pairing connection does not start the project listener.", style = MaterialTheme.typography.caption)
                        } else if (document.document != null && document.sessionRole?.isHost == false) {
                            Text(if (remote.tether.isBlank()) "Enter the other device's USB project address in Other connection options." else "Other device USB project address configured; review it in Other connection options.", style = MaterialTheme.typography.caption)
                            if (document.sessionRole?.isHost == false) Button(onClick = { chosenPeer?.let { editor.connectSession(sessions, it.deviceId, remote.endpoints(), false, chosenAddress?.interfaceIndex) } }, enabled = projectBlocker == null) { Text("Connect replica to host") }
                            Button(onClick = { chosenPeer?.let { editor.receiveProject(sessions, it.deviceId, remote.endpoints(), expectedProject, chosenAddress?.interfaceIndex) } }, enabled = receiveBlocker == null) { Text("Receive as a new local project") }
                            Text("Receiving saves a complete authenticated copy, with a 2 GiB transfer bound. Existing project folders are preserved.", style = MaterialTheme.typography.caption)
                        }
                        if (document.document != null && document.sessionRole == null) {
                            TextButton(onClick = { editor.inspectSessionRole(sessions) }, enabled = connection.ready && !connection.pairing && !document.busy) { Text("Retry project role check") }
                        }
                        projectBlocker?.let { Text(projectBlockerMessage(it, document.sessionRole?.isHost), style = MaterialTheme.typography.caption) }
                        if (projectEndpoints.isNotEmpty()) Text("Configured routes: ${projectEndpoints.joinToString { when (it.carrier) { SessionCarrier.QuicTether -> "USB tethering"; SessionCarrier.QuicWifi -> "Wi-Fi"; SessionCarrier.TcpAdb -> "developer ADB" } }}. Supplied routes retain this priority.", style = MaterialTheme.typography.caption)
                        document.sync?.let { Text("Project connection: ${it.status} · ${when (it.carrier) { SessionCarrier.QuicTether -> "USB tethering"; SessionCarrier.QuicWifi -> "Wi-Fi"; SessionCarrier.TcpAdb -> "developer ADB"; null -> "no active carrier" }}", style = MaterialTheme.typography.body2) }
                        Text(if (document.sync?.carrier == SessionCarrier.QuicTether && document.sync?.status in setOf(SyncStatus.Syncing, SyncStatus.Synced)) "USB link active. Open your Paint document, close this dialog, then Workbench → Remote Krita / Paint… → select the exact Paint window → Grant selected window. If focus is required, bring that Paint window forward and grant again." else "Connect the Workbench project over USB first. Then close this dialog and choose Workbench → Remote Krita / Paint….", style = MaterialTheme.typography.caption)
                        Text("Phone: Receive computer project opens the canvas and starts connecting. Tap More → Connect → Remote Paint, then Request PC control when available. This PC selects and grants the window.", style = MaterialTheme.typography.caption)
                        TextButton(onClick = editor::disconnectSession, enabled = document.sync != null && !document.busy) { Text("Disconnect project") }
                        if (document.busy) TextButton(onClick = editor::cancelSessionWork) { Text("Cancel connection request") }
                        if (document.background == null) document.backgroundIssue?.let { Text("Image display: $it Close this connection sheet to use the image recovery controls.", color = MaterialTheme.colors.secondary) }
                        document.message?.let { Text("Current project status: $it", color = MaterialTheme.colors.secondary) }
                        if (document.routeWarnings.isNotEmpty()) {
                            Text("Tether default-route warning", color = MaterialTheme.colors.secondary)
                            Text("The tether may be carrying PC internet traffic. Review a fresh proposal in Other connection options; Apply and Revert each require an explicit administrator action.")
                            for (finding in document.routeWarnings) {
                                Text("${finding.family} · interface ${finding.interfaceIndex} · ${finding.risk}")
                                val family = when (finding.family.lowercase()) { "ipv4" -> MetricFamily.Ipv4; "ipv6" -> MetricFamily.Ipv6; else -> null }
                                if (family != null && finding.fixCommand != null) TextButton(onClick = { otherOptions = true; assistance.prepareMetric(finding.interfaceIndex, family) }) { Text("Review metric fix") }
                            }
                        }
                    }
                }
                Divider()
                TextButton(onClick = { otherOptions = !otherOptions }) { Text(if (otherOptions) "Hide other connection options" else "Other connection options ▾") }
                if (connection.discovering && !otherOptions) Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Nearby endpoint discovery is active.", Modifier.padding(top = 12.dp))
                    TextButton(onClick = sessions::stopDiscovery) { Text("Stop finding") }
                }
                if (assistanceState.watching || assistanceState.busy || assistanceState.reverts.isNotEmpty() || assistanceState.recoveries.isNotEmpty()) Text("Connection assistance has an active request, recovery watch, or saved metric recovery. Open Other connection options to review or stop it.", color = MaterialTheme.colors.secondary)
                if (otherOptions) {
                    TextButton(onClick = { computerDetails = !computerDetails }) { Text(if (computerDetails) "Hide computer identity details" else "Computer identity details") }
                    if (computerDetails) connection.local?.let { localDevice -> SelectionContainer { Column {
                        Text("Saved PC device ID: ${localDevice.deviceId}", style = MaterialTheme.typography.caption)
                        Text("Complete PC fingerprint: ${localDevice.fingerprint}", style = MaterialTheme.typography.caption)
                    } } }
                    Text("Local developer connections", style = MaterialTheme.typography.subtitle1)
                    for (address in addresses.filter { it.loopback }) TextButton(onClick = { changeDraft { it.selectConnection(address) } }, enabled = !connection.pairing && !refreshingAddresses) { Text(connectionLabel(address, addresses)) }
                    Text("Manual pairing address", style = MaterialTheme.typography.subtitle1)
                    OutlinedTextField(bind, { value -> changeDraft { it.copy(bind = value.take(128)) } }, enabled = !connection.pairing, label = { Text("Local numeric pairing address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                    Text("This changes the pairing listener only. Choose the appropriate PC interface above separately for a USB project connection.", style = MaterialTheme.typography.caption)
                    if (screenPath == ConnectionScreenPath.PairedProject) {
                        Text("Local project listeners", style = MaterialTheme.typography.subtitle1)
                        OutlinedTextField(local.tether, { value -> changeDraft { it.manualLocalTether(value.take(128)) } }, label = { Text("PC USB session address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                        AlternativeEndpointEditor(local, { change -> changeDraft { it.copy(local = change(it.local)) } })
                        TextButton(onClick = { changeDraft { it.copy(local = it.local.copy(wifi = it.bind)) } }, enabled = bind.isNotBlank()) { Text("Use pairing address for the Wi-Fi project listener") }
                        Text("Other device project endpoints", style = MaterialTheme.typography.subtitle1)
                        OutlinedTextField(remote.tether, { value -> changeDraft { it.copy(remote = it.remote.copy(tether = value.take(128))) } }, label = { Text("Other device USB project address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                        AlternativeEndpointEditor(remote, { change -> changeDraft { it.copy(remote = change(it.remote)) } })
                        OutlinedTextField(expectedProject, { value -> changeDraft { it.copy(expectedProject = value.take(64)) } }, label = { Text("Expected project ID (optional)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                        if (document.sessionRole?.isHost == true || document.document == null) {
                            Button(onClick = { chosenPeer?.let { editor.receiveProject(sessions, it.deviceId, remote.endpoints(), expectedProject, chosenAddress?.interfaceIndex) } }, enabled = receiveBlocker == null) { Text("Receive as a new local project") }
                            Text("Receiving saves a complete authenticated copy, with a 2 GiB transfer bound. Existing project folders are preserved.", style = MaterialTheme.typography.caption)
                        }
                        Row { Checkbox(document.shareViewport, editor::shareViewport); Text("Share my viewport", Modifier.padding(top = 12.dp)) }
                        Text("Peer edits keep your camera unchanged. Follow and match-view remain explicit controls.", style = MaterialTheme.typography.caption)
                    }
                    Divider()
                    Text("Nearby endpoints", style = MaterialTheme.typography.subtitle1)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { chosenAddress?.let { sessions.browse(listOf(it.address)) } }, enabled = connection.ready && chosenAddress != null && !connection.discovering) { Text("Find nearby endpoints") }
                        if (connection.discovering) TextButton(onClick = sessions::stopDiscovery) { Text("Stop finding") }
                    }
                    for ((number, found) in connection.discovered.withIndex()) for ((route, address) in found.endpoints.withIndex()) {
                        TextButton(onClick = { changeDraft { it.copy(peerEndpoint = address, remote = it.remote.copy(wifi = address), method = PairingMethod.Join) }; requestedNewPhone = true; otherOptions = false }, enabled = !connection.pairing) { Text("Use nearby Workbench ${number + 1}${if (found.endpoints.size > 1) " · connection ${route + 1}" else ""}") }
                        if (connectionDetails) SelectionContainer { Text(address, style = MaterialTheme.typography.caption) }
                    }
                    Text("Nearby endpoints are untrusted hints. Pairing still verifies the certificate and secret.", style = MaterialTheme.typography.caption)
                    Divider()
                    Text("Manage paired devices", style = MaterialTheme.typography.subtitle1)
                    TextButton(onClick = sessions::refreshTrust, enabled = connection.ready && !connection.pairing) { Text("Refresh paired devices") }
                    for (device in connection.paired) {
                        Text("${phoneName(device, connection)}${if (device.revokedAtMs != null) " · revoked" else ""}")
                        SelectionContainer { Text(device.fingerprint, style = MaterialTheme.typography.caption) }
                        if (device.revokedAtMs == null) TextButton(onClick = { sessions.revoke(device.deviceId, editor::disconnectPeer) }) { Text("Revoke pairing") }
                    }
                    Divider()
                    if (screenPath == ConnectionScreenPath.PairedProject || assistanceState.watching || assistanceState.busy || assistanceState.reverts.isNotEmpty() || assistanceState.recoveries.isNotEmpty()) {
                        ConnectionAssistancePanel(assistance, chosenAddress?.interfaceIndex, chooseTool = {
                            val picker = java.awt.FileDialog(window, "Choose your installed adb.exe", java.awt.FileDialog.LOAD)
                            try { picker.file = "adb.exe"; picker.isVisible = true
                                val name = picker.file
                                if (name != null) assistance.selectTool(java.nio.file.Path.of(picker.directory, name).toAbsolutePath().normalize().toString())
                            } finally { picker.dispose() }
                        }, useListener = { value -> changeDraft { it.copy(local = it.local.copy(adb = value)) } }, phonePort = phonePort, changePhonePort = { value -> changeDraft { it.copy(phonePort = value) } }, hostPort = hostPort, changeHostPort = { value -> changeDraft { it.copy(hostPort = value) } })
                        Text("Firewall and physical connection checks still need platform validation.", style = MaterialTheme.typography.caption)
                    }
                }
                TextButton(onClick = dismiss) { Text("Close") }
            }
        }
    }
}

@Composable private fun AlternativeEndpointEditor(fields: EndpointFields, changed: ((EndpointFields) -> EndpointFields) -> Unit) {
    OutlinedTextField(fields.wifi, { value -> changed { it.copy(wifi = value.take(128)) } }, label = { Text("Wi-Fi project address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
    OutlinedTextField(fields.adb, { value -> changed { it.copy(adb = value.take(128)) } }, label = { Text("Developer ADB TCP loopback address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
}

private fun projectBlockerMessage(blocker: ProjectConnectionBlocker, hosting: Boolean?): String = when (blocker) {
    ProjectConnectionBlocker.PairingStorage -> "Project connections need protected pairing storage to be ready."
    ProjectConnectionBlocker.PairingActive -> "Finish or cancel pairing before connecting a project."
    ProjectConnectionBlocker.ProjectBusy -> "Wait for the current project operation, or cancel its connection request below."
    ProjectConnectionBlocker.SelectPeer -> "Select a currently paired phone before connecting a project."
    ProjectConnectionBlocker.OpenProject -> "Open or import a Workbench image before hosting this PC's connection."
    ProjectConnectionBlocker.ProjectRole -> "Check the current project status, then choose Retry project role check. Sharing needs verified project ownership."
    ProjectConnectionBlocker.OriginalHost -> "Select this replica's original paired host. The project's ownership is unchanged."
    ProjectConnectionBlocker.RefreshingConnections -> "Wait for PC connections to finish refreshing."
    ProjectConnectionBlocker.SelectConnection -> "Choose the PC tether connection above so its default-route safety can be checked."
    ProjectConnectionBlocker.PairingEndpointMismatch -> "The manual pairing address does not match the selected PC connection. Choose the connection again, or enter the USB project listener in Other connection options."
    ProjectConnectionBlocker.ProjectEndpoint -> if (hosting == true) "Choose Use selected connection for USB project, or enter a project listener in Other connection options." else "Enter the other device's project address in Other connection options."
}

private fun phoneName(device: PairedDevice, connection: DesktopSessionState): String {
    val ordered = connection.paired.sortedWith(compareBy({ it.pairedAtMs }, { it.deviceId }, { it.fingerprint }))
    fun base(peer: PairedDevice): String = connection.labels?.nameFor(peer.deviceId, peer.fingerprint)
        ?: if (ordered.size == 1) "My phone" else "Phone ${ordered.indexOf(peer) + 1}"
    val name = base(device)
    return if (ordered.count { base(it).equals(name, ignoreCase = true) } > 1)
        "$name · ${device.fingerprint.take(8)} · ${ordered.indexOf(device) + 1}" else name
}

private fun connectionLabel(address: LocalSessionAddress, addresses: List<LocalSessionAddress>): String =
    networkConnectionLabel(address, addresses)

@Composable private fun PcConnectionPicker(addresses: List<LocalSessionAddress>, selected: LocalSessionAddress?,
    refreshing: Boolean, canChange: Boolean, refresh: () -> Unit, choose: (LocalSessionAddress) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    LaunchedEffect(canChange, refreshing) { if (!canChange || refreshing) expanded = false }
    Text("PC network connection", style = MaterialTheme.typography.subtitle1)
    Text("Choose the adapter linked to the phone. For Paint, enable USB tethering on the phone and choose its PC adapter. An adapter name alone does not prove a USB link.", style = MaterialTheme.typography.caption)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.weight(1f)) {
            OutlinedButton(onClick = { expanded = true }, enabled = canChange && !refreshing, modifier = Modifier.fillMaxWidth()) {
                Column {
                    Text(selected?.let { adapterLabel(it, addresses) } ?: "Choose PC network connection…")
                    selected?.let { Text(addressOptionLabel(it, addresses), style = MaterialTheme.typography.caption) }
                }
            }
            DropdownMenu(expanded = expanded && canChange && !refreshing, onDismissRequest = { expanded = false }) {
                for (address in addresses.filterNot { it.loopback }) DropdownMenuItem(onClick = { choose(address); expanded = false }) {
                    Column {
                        Text(adapterLabel(address, addresses))
                        Text(addressOptionLabel(address, addresses), style = MaterialTheme.typography.caption)
                    }
                }
                if (addresses.none { !it.loopback }) DropdownMenuItem(onClick = { expanded = false }, enabled = false) { Text("No PC network connections available") }
            }
        }
        TextButton(onClick = refresh, enabled = canChange && !refreshing) { Text("Refresh") }
    }
    if (refreshing) Text("Refreshing PC network connections…", style = MaterialTheme.typography.caption)
    selected?.let { Text("Selected PC adapter: ${adapterLabel(it, addresses)}. The paired phone is chosen separately by its saved name.", style = MaterialTheme.typography.caption) }
}

@Composable private fun PairingConnectionDetails(address: LocalSessionAddress?, bind: String, expanded: Boolean, toggle: () -> Unit) {
    TextButton(onClick = toggle) { Text(if (expanded) "Hide network details" else "Network details") }
    if (expanded) SelectionContainer { Column {
        address?.let { Text("PC adapter: ${it.interfaceName ?: "Unnamed local adapter"} · interface ${it.interfaceIndex}\nNumeric address: ${it.address}", style = MaterialTheme.typography.caption) }
        if (bind.isNotBlank()) Text("Pairing listener: $bind", style = MaterialTheme.typography.caption)
    } }
}

private fun pairingBlockerMessage(blocker: PairingActionBlocker): String = when (blocker) {
    PairingActionBlocker.Storage -> "Wait for protected pairing storage to open."
    PairingActionBlocker.Active -> "Wait for the current pairing request, complete its fingerprint comparison, or cancel before starting another offer."
    PairingActionBlocker.Refreshing -> "Wait for PC network connections to finish refreshing."
    PairingActionBlocker.LocalConnection -> "Choose the PC network connection above to enable Show, or enter its numeric listener in Other connection options."
    PairingActionBlocker.OtherEndpoint -> "Enter the pairing address displayed by the other device."
    PairingActionBlocker.Code -> "Enter all eight digits from the other device."
}

@Composable private fun QrCode(text: String) {
    var matrix by remember(text) { mutableStateOf<BitMatrix?>(null) }
    var failed by remember(text) { mutableStateOf(false) }
    val reveal = remember(text) { BringIntoViewRequester() }
    LaunchedEffect(text) {
        var generated: BitMatrix? = null
        try {
            generated = withContext(Dispatchers.Default) { QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 1, 1,
                mapOf(EncodeHintType.CHARACTER_SET to "UTF-8", EncodeHintType.MARGIN to 4)) }
            matrix = generated; generated = null
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) { failed = true }
        finally { generated?.clear() }
    }
    DisposableEffect(text) { onDispose { matrix?.clear(); matrix = null } }
    LaunchedEffect(text, matrix) {
        if (matrix != null) { withFrameNanos { }; reveal.bringIntoView() }
    }
    if (failed) Text("This QR could not be displayed. Cancel pairing, then choose Eight-digit code.", color = MaterialTheme.colors.secondary)
    else if (matrix == null) {
        Text("Preparing the QR image…", style = MaterialTheme.typography.caption)
        LinearProgressIndicator(Modifier.fillMaxWidth())
    } else BoxWithConstraints(Modifier.widthIn(max = 320.dp).fillMaxWidth().bringIntoViewRequester(reveal)) {
        val dimension = minOf(maxWidth, 320.dp)
        val bits = matrix
        if (bits != null && dimension.value * LocalDensity.current.density < bits.width)
            Text("Widen this window to show the complete QR, or cancel and use the eight-digit code.")
        else Column {
            Text("QR ready to scan", style = MaterialTheme.typography.subtitle1)
            Canvas(Modifier.size(dimension)) {
                drawRect(Color.White)
                bits?.let {
                    val scale = kotlin.math.floor(minOf(size.width / it.width, size.height / it.height))
                    val left = kotlin.math.floor((size.width - it.width * scale) / 2)
                    val top = kotlin.math.floor((size.height - it.height * scale) / 2)
                    for (y in 0 until it.height) for (x in 0 until it.width) if (it[x, y])
                        drawRect(Color.Black, Offset(left + x * scale, top + y * scale), Size(scale, scale))
                }
            }
        }
    }
}
