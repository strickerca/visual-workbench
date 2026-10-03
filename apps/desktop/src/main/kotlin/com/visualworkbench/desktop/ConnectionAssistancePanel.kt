package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
internal fun ConnectionAssistancePanel(controller: ConnectionAssistanceController, selectedInterface: UInt?, chooseTool: () -> Unit,
    useListener: (String) -> Unit) {
    val state by controller.state.collectAsState()
    var phonePort by remember { mutableStateOf("47191") }
    var hostPort by remember { mutableStateOf("47191") }
    Text("Developer USB connection", style = MaterialTheme.typography.h6)
    Text("Choose your installed ADB tool and one device. Recovery uses the existing local server; it never starts or resets ADB, changes another mapping, or touches another device.", style = MaterialTheme.typography.body2)
    TextButton(onClick = chooseTool, enabled = !state.busy && !state.watching) { Text("Choose installed adb.exe…") }
    if (state.adbPath.isNotBlank()) Text(state.adbPath, style = MaterialTheme.typography.caption)
    Button(onClick = controller::inspectDevices, enabled = state.adbPath.isNotBlank() && !state.busy && !state.watching) { Text("Inspect connected devices") }
    for (device in state.devices) {
        Row {
            RadioButton(state.selectedSerial == device.serial, onClick = { controller.selectDevice(device.serial) }, enabled = !state.busy && !state.watching)
            Text("${device.serial} · ${device.state}", Modifier.padding(top = 12.dp))
        }
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedTextField(phonePort, { phonePort = it.filter(Char::isDigit).take(5) }, label = { Text("Phone TCP port") }, singleLine = true,
            enabled = !state.busy && !state.watching, modifier = Modifier.weight(1f))
        OutlinedTextField(hostPort, { hostPort = it.filter(Char::isDigit).take(5) }, label = { Text("PC TCP listener port") }, singleLine = true,
            enabled = !state.busy && !state.watching, modifier = Modifier.weight(1f))
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Button(onClick = { controller.startReverse(phonePort, hostPort) }, enabled = state.selectedSerial != null && !state.busy && !state.watching) { Text("Enable mapping recovery") }
        TextButton(onClick = controller::stopReverse, enabled = state.watching && !state.busy) { Text("Stop recovery") }
    }
    state.reverse?.let { reverse ->
        Text(reverseLabel(reverse), color = MaterialTheme.colors.secondary)
        Text("Phone 127.0.0.1:${reverse.phonePort} → PC 127.0.0.1:${reverse.hostPort}", style = MaterialTheme.typography.caption)
        if (reverse.state == ReverseState.MappingCreated || reverse.state == ReverseState.ExistingMapping)
            TextButton(onClick = { useListener("127.0.0.1:${reverse.hostPort}") }) { Text("Use this PC listener for the project") }
        if (reverse.mappingMayRemain) Text("This watch created ${reverse.created} mapping(s). They may remain after Stop or app exit.", style = MaterialTheme.typography.caption)
    }
    Text("Stopping recovery leaves mappings in place because ADB cannot prove that another app has not adopted them. Remove an unwanted mapping through your own ADB tool. Recovery scheduling targets under one second; physical replug timing remains unverified.", style = MaterialTheme.typography.caption)
    Divider()
    Text("Tether route protection", style = MaterialTheme.typography.h6)
    Text("Inspect a selected interface, then review its old and proposed metrics. Applying or reverting asks Windows for administrator approval. A changed route table cancels the proposal.", style = MaterialTheme.typography.body2)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(onClick = { selectedInterface?.let { controller.prepareMetric(it, MetricFamily.Ipv4) } }, enabled = selectedInterface != null && !state.busy) { Text("Inspect IPv4 metric") }
        TextButton(onClick = { selectedInterface?.let { controller.prepareMetric(it, MetricFamily.Ipv6) } }, enabled = selectedInterface != null && !state.busy) { Text("Inspect IPv6 metric") }
    }
    state.metric?.let { metric ->
        Text("${metric.family} · interface ${metric.interfaceIndex}")
        Text("Current: ${if (metric.oldAutomatic) "automatic" else "manual"} metric ${metric.oldMetric}")
        Text("Proposed: ${if (metric.newAutomatic) "automatic (Windows may recalculate)" else "manual"} metric ${metric.newMetric}")
        Button(onClick = controller::applyMetric, enabled = !state.busy) { Text(if (metric.kind == MetricKind.Revert) "Revert metric · administrator…" else "Apply metric fix · administrator…") }
        Text("The approval request expires after 45 seconds. No metric is automatically reverted on disconnect or exit.", style = MaterialTheme.typography.caption)
    }
    for (inverse in state.reverts.asReversed()) {
        Text("Revert ${inverse.family} interface ${inverse.interfaceIndex}: ${if (inverse.oldAutomatic) "automatic" else "manual"} ${inverse.oldMetric} → ${if (inverse.newAutomatic) "automatic" else "manual"} ${inverse.newMetric}")
        Button(onClick = { controller.revertMetric(inverse.interfaceIndex, inverse.family) }, enabled = !state.busy && inverse == state.reverts.last()) { Text("Revert this metric · administrator…") }
    }
    if (state.reverts.isNotEmpty()) Text("Revert the most recent change first. Receipts remain available during this app session; unrelated route changes can make them stale. No revert runs automatically.", style = MaterialTheme.typography.caption)
    for (recovery in state.recoveries) {
        Text("Saved original ${recovery.family} interface ${recovery.interfaceIndex}: ${if (recovery.originalAutomatic) "automatic" else "manual"} metric ${recovery.originalMetric}", color = MaterialTheme.colors.secondary)
        Text("The last outcome was stale or uncertain. These values remain for recovery; inspect Windows routing before restoring them. No change or successful revert is implied.", style = MaterialTheme.typography.caption)
        TextButton(onClick = { controller.resolveRecovery(recovery.interfaceIndex, recovery.family) }, enabled = !state.busy) { Text("I resolved this · dismiss saved values") }
    }
    if (state.busy) {
        LinearProgressIndicator(Modifier.fillMaxWidth())
        TextButton(onClick = controller::cancelWork) { Text("Cancel this request") }
    }
    state.message?.let { Text(it, color = MaterialTheme.colors.secondary) }
}
