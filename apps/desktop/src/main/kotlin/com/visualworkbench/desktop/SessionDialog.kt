package com.visualworkbench.desktop

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindow
import androidx.compose.ui.window.rememberDialogState
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.common.BitMatrix
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

/** Immutable fields captured before an operation starts. Unselected carriers
 * are omitted; supplied carriers retain the documented tether/Wi-Fi/adb order. */
internal data class EndpointFields(val tether: String = "", val wifi: String = "", val adb: String = "") {
    fun endpoints(): List<SessionEndpoint> = listOf(SessionCarrier.QuicTether to tether,
        SessionCarrier.QuicWifi to wifi, SessionCarrier.TcpAdb to adb)
        .filter { it.second.isNotBlank() }.map { SessionEndpoint(it.first, it.second.trim()) }
}

@Composable
internal fun SessionDialog(sessions: DesktopSessionController, editor: EditorController, assistance: ConnectionAssistanceController, dismiss: () -> Unit) {
    val connection by sessions.state.collectAsState()
    val document by editor.state.collectAsState()
    var addresses by remember { mutableStateOf<List<LocalSessionAddress>>(emptyList()) }
    var addressError by remember { mutableStateOf<String?>(null) }
    var selectedAddress by remember { mutableStateOf<LocalSessionAddress?>(null) }
    var bind by remember { mutableStateOf("") }
    var peerEndpoint by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var peer by remember { mutableStateOf<String?>(null) }
    var local by remember { mutableStateOf(EndpointFields()) }
    var remote by remember { mutableStateOf(EndpointFields()) }
    var expectedProject by remember { mutableStateOf("") }
    var compared by remember(connection.fingerprint) { mutableStateOf(false) }
    var now by remember { mutableStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        sessions.refreshTrust()
        editor.inspectSessionRole(sessions)
        try { addresses = localSessionAddresses() }
        catch (_: Exception) { addressError = "Local address enumeration is unavailable. Enter a known numeric local address and port to pair over Wi-Fi." }
    }
    LaunchedEffect(connection.display) { while (connection.display != null) { now = System.currentTimeMillis(); delay(1000) } }
    DisposableEffect(Unit) { onDispose { code = ""; sessions.cancelPairing(); sessions.stopDiscovery() } }
    DialogWindow(onCloseRequest = dismiss, title = "Devices and live projects", state = rememberDialogState(width = 760.dp, height = 800.dp)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Devices and live projects", style = MaterialTheme.typography.h5)
                Text("Choose a local address before opening a listener. Pairing and project connections use only the endpoints you select.", style = MaterialTheme.typography.body2)
                if (!connection.ready) Text(connection.message ?: "Opening protected pairing storage…")
                addressError?.let { Text(it) }
                for (address in addresses) {
                    TextButton(onClick = { selectedAddress = address; bind = endpoint(address.address, 44242) }, enabled = !connection.pairing) {
                        Text("${if (selectedAddress == address) "✓ " else ""}${address.address} · interface ${address.interfaceIndex}")
                    }
                }
                OutlinedTextField(bind, { bind = it.take(128) }, enabled = !connection.pairing, label = { Text("Local pairing address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { sessions.offer(bind, false) }, enabled = connection.ready && !connection.pairing && bind.isNotBlank()) { Text("Show QR") }
                    OutlinedButton(onClick = { sessions.offer(bind, true) }, enabled = connection.ready && !connection.pairing && bind.isNotBlank()) { Text("Show eight-digit code") }
                    if (connection.pairing) TextButton(onClick = sessions::cancelPairing) { Text("Cancel pairing") }
                }
                connection.display?.let { display ->
                    Text("Scan with the other device. This offer expires in ${((display.offer.expiresAtMs.toLong() - now).coerceAtLeast(0) + 999) / 1000} seconds.")
                    display.qrText?.let { QrCode(it) }
                    if (display.offer.code.isNotEmpty()) Text(display.offer.code, style = MaterialTheme.typography.h4)
                    Text("Pairing endpoint: ${display.offer.endpoint}", style = MaterialTheme.typography.caption)
                }
                connection.fingerprint?.let { fingerprint ->
                    Text("Compare this complete fingerprint on both devices before confirming.", style = MaterialTheme.typography.subtitle1)
                    SelectionContainer { Text(fingerprint.chunked(8).joinToString(" ")) }
                    Row { Checkbox(compared, { compared = it }, enabled = !connection.confirming); Text("The complete fingerprints match on both devices.", Modifier.padding(top = 12.dp)) }
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { sessions.confirmFingerprint(fingerprint, compared) }, enabled = compared && !connection.confirming) { Text("Confirm pairing") }
                        TextButton(onClick = sessions::decline, enabled = !connection.confirming) { Text("Decline") }
                    }
                }
                Divider()
                Text("Join using a code", style = MaterialTheme.typography.h6)
                OutlinedTextField(peerEndpoint, { peerEndpoint = it.take(128) }, label = { Text("Other device address:port") }, singleLine = true, enabled = !connection.pairing, modifier = Modifier.fillMaxWidth())
                OutlinedTextField(code, { code = it.filter(Char::isDigit).take(8) }, label = { Text("Eight-digit code") }, singleLine = true, enabled = !connection.pairing)
                Button(onClick = { val entered = code; code = ""; sessions.joinCode(entered, peerEndpoint) }, enabled = connection.ready && !connection.pairing && code.length == 8 && peerEndpoint.isNotBlank()) { Text("Join and compare fingerprints") }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { selectedAddress?.let { sessions.browse(listOf(it.address)) } }, enabled = connection.ready && selectedAddress != null && !connection.discovering) { Text("Find nearby endpoints") }
                    if (connection.discovering) TextButton(onClick = sessions::stopDiscovery) { Text("Stop finding") }
                }
                for (found in connection.discovered) for (address in found.endpoints) {
                    TextButton(onClick = { peerEndpoint = address; remote = remote.copy(wifi = address) }) { Text("Use nearby endpoint $address") }
                }
                if (connection.discovering) Text("Nearby endpoints are untrusted hints. Pairing still verifies the certificate and secret.", style = MaterialTheme.typography.caption)
                Divider()
                Text("Paired devices", style = MaterialTheme.typography.h6)
                if (connection.paired.isEmpty()) Text("No paired devices yet.")
                for (device in connection.paired) {
                    Column {
                        Row {
                            RadioButton(peer == device.deviceId, onClick = { peer = device.deviceId }, enabled = device.revokedAtMs == null)
                            Text("Device ${device.deviceId.takeLast(8)}${if (device.revokedAtMs != null) " · revoked" else ""}", Modifier.padding(top = 12.dp))
                            if (device.revokedAtMs == null) TextButton(onClick = { sessions.revoke(device.deviceId, editor::disconnectPeer) }) { Text("Revoke") }
                        }
                        SelectionContainer { Text(device.fingerprint, style = MaterialTheme.typography.caption) }
                    }
                }
                Divider()
                Text("Current project", style = MaterialTheme.typography.h6)
                Text(document.document?.title ?: "No project open")
                Text(document.sessionRole?.let { if (it.isHost) "This device hosts the project journal." else "Local replica · ${it.pending} pending · ${it.blocked} blocked" } ?: "Choose a project or receive a new local replica.")
                Text("Local listeners", style = MaterialTheme.typography.subtitle1)
                EndpointEditor(local, { local = it })
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { local = local.copy(tether = bind) }, enabled = bind.isNotBlank()) { Text("Use pairing address for tether") }
                    TextButton(onClick = { local = local.copy(wifi = bind) }, enabled = bind.isNotBlank()) { Text("Use it for Wi-Fi") }
                }
                Button(onClick = { peer?.let { editor.connectSession(sessions, it, local.endpoints(), true, selectedAddress?.interfaceIndex) } }, enabled = connection.ready && peer != null && !connection.pairing && !document.busy && document.sessionRole?.isHost == true && local.endpoints().isNotEmpty()) { Text("Share project") }
                Text("Peer endpoints", style = MaterialTheme.typography.subtitle1)
                EndpointEditor(remote, { remote = it })
                Button(onClick = { peer?.let { editor.connectSession(sessions, it, remote.endpoints(), false, selectedAddress?.interfaceIndex) } }, enabled = connection.ready && peer != null && !connection.pairing && !document.busy && document.sessionRole?.isHost == false && remote.endpoints().isNotEmpty()) { Text("Connect replica to host") }
                OutlinedTextField(expectedProject, { expectedProject = it.take(64) }, label = { Text("Expected project ID (optional)") }, singleLine = true, modifier = Modifier.fillMaxWidth())
                Button(onClick = { peer?.let { editor.receiveProject(sessions, it, remote.endpoints(), expectedProject, selectedAddress?.interfaceIndex) } }, enabled = connection.ready && peer != null && !connection.pairing && !document.busy && remote.endpoints().isNotEmpty()) { Text("Receive as a new local project") }
                Text("Receiving saves a complete authenticated copy, with a 2 GiB transfer bound. Existing project folders are preserved.", style = MaterialTheme.typography.caption)
                TextButton(onClick = editor::disconnectSession, enabled = document.sync != null && !document.busy) { Text("Disconnect project") }
                if (document.busy) TextButton(onClick = editor::cancelSessionWork) { Text("Cancel connection request") }
                Row { Checkbox(document.shareViewport, editor::shareViewport); Text("Share my viewport", Modifier.padding(top = 12.dp)) }
                Text("Peer edits keep your camera unchanged. Follow and match-view remain explicit controls.", style = MaterialTheme.typography.caption)
                if (document.routeWarnings.isNotEmpty()) {
                    Text("Tether default-route warning", color = MaterialTheme.colors.secondary)
                    Text("The tether may be carrying PC internet traffic. Review a fresh proposal below; Apply and Revert each require an explicit administrator action.")
                    for (finding in document.routeWarnings) {
                        Text("${finding.family} · interface ${finding.interfaceIndex} · ${finding.risk}")
                        val family = when (finding.family.lowercase()) { "ipv4" -> MetricFamily.Ipv4; "ipv6" -> MetricFamily.Ipv6; else -> null }
                        if (family != null && finding.fixCommand != null) TextButton(onClick = { assistance.prepareMetric(finding.interfaceIndex, family) }) { Text("Review metric fix") }
                    }
                }
                ConnectionAssistancePanel(assistance, selectedAddress?.interfaceIndex, chooseTool = {
                    val picker = java.awt.FileDialog(window, "Choose your installed adb.exe", java.awt.FileDialog.LOAD)
                    try { picker.file = "adb.exe"; picker.isVisible = true
                        val name = picker.file
                        if (name != null) assistance.selectTool(java.nio.file.Path.of(picker.directory, name).toAbsolutePath().normalize().toString())
                    } finally { picker.dispose() }
                }, useListener = { local = local.copy(adb = it) })
                Text("Firewall and physical connection checks still need platform validation.", style = MaterialTheme.typography.caption)
                connection.message?.let { Text(it, color = MaterialTheme.colors.secondary) }
                document.message?.let { Text(it, color = MaterialTheme.colors.secondary) }
                TextButton(onClick = dismiss) { Text("Close") }
            }
        }
    }
}

@Composable private fun EndpointEditor(fields: EndpointFields, changed: (EndpointFields) -> Unit) {
    OutlinedTextField(fields.tether, { changed(fields.copy(tether = it.take(128))) }, label = { Text("USB tether address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
    OutlinedTextField(fields.wifi, { changed(fields.copy(wifi = it.take(128))) }, label = { Text("Wi-Fi address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
    OutlinedTextField(fields.adb, { changed(fields.copy(adb = it.take(128))) }, label = { Text("adb TCP loopback address:port") }, singleLine = true, modifier = Modifier.fillMaxWidth())
}

private fun endpoint(address: String, port: Int): String = if (':' in address) "[$address]:$port" else "$address:$port"

@Composable private fun QrCode(text: String) {
    var matrix by remember(text) { mutableStateOf<BitMatrix?>(null) }
    var failed by remember(text) { mutableStateOf(false) }
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
    if (failed) Text("This QR could not be displayed. Cancel it and choose the eight-digit code.")
    else Canvas(Modifier.size(320.dp)) {
        drawRect(Color.White)
        matrix?.let { bits ->
            val scale = kotlin.math.floor(minOf(size.width / bits.width, size.height / bits.height)).coerceAtLeast(1f)
            val left = (size.width - bits.width * scale) / 2; val top = (size.height - bits.height * scale) / 2
            for (y in 0 until bits.height) for (x in 0 until bits.width) if (bits[x, y])
                drawRect(Color.Black, Offset(left + x * scale, top + y * scale), Size(scale, scale))
        }
    }
}
