package com.visualworkbench.android.editor

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

/** No effect sends, retries, initializes history or reads a credential. Those
 * actions have separate buttons; Send captures this exact displayed review. */
@Composable
internal fun AiWorkspace(controller: AiEditorController<AiBitmap>, service: AiServiceController,
    core: WorkbenchCore, onKeys: () -> Unit, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    val state by controller.state.collectAsState()
    val settings by service.state.collectAsState()
    Surface(modifier.fillMaxSize(), elevation = 12.dp) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) { Text("AI image edit", style = MaterialTheme.typography.h6)
                    Text("Review → explicit Send → compare → save Result", style = MaterialTheme.typography.caption) }
                TextButton({ controller.cancel() }, enabled = !state.idle || state.loadingPixels) { Text("Cancel operation") }
                TextButton(onDismiss) { Text("Close") }
            }
            BoxWithConstraints(Modifier.weight(1f)) {
                if (maxWidth > 760.dp) Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    AiControls(controller, service, state, settings, onKeys, Modifier.width(360.dp).fillMaxHeight())
                    AiCompareCanvas(controller, core, state, Modifier.weight(1f).fillMaxHeight())
                } else Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    AiCompareCanvas(controller, core, state, Modifier.fillMaxWidth().weight(1f))
                    AiControls(controller, service, state, settings, onKeys, Modifier.fillMaxWidth().weight(1.2f))
                }
            }
        }
    }
}

@Composable
private fun AiControls(controller: AiEditorController<AiBitmap>, service: AiServiceController,
    state: AiEditorState<AiBitmap>, settings: AiServiceState, onKeys: () -> Unit, modifier: Modifier) {
    var showSettings by remember { mutableStateOf(false) }
    var instruction by remember { mutableStateOf("") }
    var textTokens by remember { mutableStateOf("") }
    var inputTokens by remember { mutableStateOf("") }
    var outputTokens by remember { mutableStateOf("") }
    var provenance by remember { mutableStateOf("") }
    var verified by remember { mutableStateOf("") }
    var expires by remember { mutableStateOf("") }
    var feather by remember { mutableStateOf(8f) }
    var depth by remember { mutableStateOf(false) }
    var untagged by remember { mutableStateOf(false) }
    var localMessage by remember { mutableStateOf<String?>(null) }
    var acknowledge by remember(state.review?.requestId) { mutableStateOf(false) }
    val enabled = state.idle && !settings.busy
    Column(modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton({ showSettings = !showSettings; if (showSettings) service.activate() }, Modifier.fillMaxWidth(), enabled = state.idle) { Text("Provider settings & budget") }
        if (showSettings) AiSettings(service, settings, onKeys, state.idle)
        state.context?.let { context ->
            Text("Source ${context.width} × ${context.height} · ${context.bitDepth}-bit · ${context.changeSelections.size} Change masks")
            Text("Native includes all visible Change masks and role-tagged instructions. Original bytes remain immutable.", style = MaterialTheme.typography.caption)
            LazyColumn(Modifier.fillMaxWidth().heightIn(max=180.dp)) {
                items(context.instructions,key={it.id}) { entry -> Text("${entry.role}: ${entry.text}",style=MaterialTheme.typography.caption) }
            }
        }
        TextButton({ controller.reloadContext() }, enabled = enabled) { Text("Refresh document context") }
        OutlinedTextField(instruction, { instruction = it.take(16_384) }, label = { Text("Additional global Change instruction (optional)") }, enabled = enabled, modifier = Modifier.fillMaxWidth())
        Text("Explicit token estimate", style = MaterialTheme.typography.subtitle2)
        Text("Use current provider-supported figures and cite their source. The app does not invent a token formula.", style = MaterialTheme.typography.caption)
        AiSmallInput("Text input tokens", textTokens, { textTokens = it.take(9) }, enabled)
        AiSmallInput("Image input tokens", inputTokens, { inputTokens = it.take(9) }, enabled)
        AiSmallInput("Image output tokens", outputTokens, { outputTokens = it.take(9) }, enabled)
        AiSmallInput("Estimate source", provenance, { provenance = it.take(1024) }, enabled)
        AiSmallInput("Verified on · YYYY-MM-DD", verified, { verified = it.take(10) }, enabled)
        AiSmallInput("Expires on · YYYY-MM-DD", expires, { expires = it.take(10) }, enabled)
        Text("Feather ${feather.toInt()} px")
        Slider(feather, { feather = it }, valueRange = 0f..64f, steps = 63, enabled = enabled)
        AiCheck("Allow an 8-bit provider copy of a 16-bit source", depth, { depth = it }, enabled)
        AiCheck("Treat an untagged source as sRGB", untagged, { untagged = it }, enabled)
        Button({
            try {
                val estimate = aiEstimateInput(textTokens, inputTokens, outputTokens, provenance, verified, expires)
                localMessage = null
                controller.prepare(AiDraftOptions(instruction, estimate, feather.toInt().toUInt(), depth, untagged))
            } catch (error: IllegalArgumentException) { localMessage = error.message }
        }, Modifier.fillMaxWidth(), enabled = enabled && settings.configuration != null && !controller.needsDecision()) { Text("Prepare request review") }
        if (controller.needsDecision()) Text("Save or explicitly discard the retained paid review before preparing a new request.", style = MaterialTheme.typography.caption)
        val review = state.review
        if (review != null) {
            Divider()
            Text("Review request", style = MaterialTheme.typography.subtitle1)
            Text("${review.model} · ${review.quality}")
            Text("${review.sourceWidth} × ${review.sourceHeight} source → ${review.modelWidth} × ${review.modelHeight} provider copy\nCrop (${review.cropX}, ${review.cropY}), ${review.cropWidth} × ${review.cropHeight}; feather ${review.featherPx} px")
            if (review.providerCopyReducesDepth) Text("The provider receives an explicitly approved 8-bit copy.")
            Text("Estimate: ${review.estimatedMicrousd?.let(::aiMoney) ?: "Unavailable — Send disabled"}")
            Text("${review.estimateProvenance.orEmpty()}\nEstimate valid through ${review.estimateExpiresOn ?: "unavailable"}; configuration through ${review.configurationExpiresOn}", style = MaterialTheme.typography.caption)
            Text("An estimate is not a provider-enforced charge cap. Canceling after Send may still incur a charge.", style = MaterialTheme.typography.caption)
            val cost = review.estimatedMicrousd
            val readiness = settings.readiness
            if (cost != null && readiness != null && aiBudgetExceeded(readiness, cost)) Text("This estimate exceeds the remaining daily soft budget.", color = MaterialTheme.colors.error)
            AiCheck("I acknowledge a request that exceeds the daily soft budget", acknowledge, { acknowledge = it }, enabled && !state.attempted)
            Button({ if (cost != null) controller.send(review.requestId, cost, acknowledge) }, Modifier.fillMaxWidth(),
                enabled = enabled && !state.stale && !state.attempted && cost != null && readiness?.trustReady == true && readiness?.configurationCurrent == true &&
                    settings.configuration?.fingerprint == review.configurationFingerprint) {
                Text(if (state.attempted) "Send attempt used" else "Send · ${cost?.let(::aiMoney).orEmpty()}")
            }
            if(state.attempted){
                TextButton({controller.recoverCandidate()},enabled=state.idle){Text("Reload paid candidate · no Send")}
                Text("Keep this project open until you save or explicitly discard the candidate. An unsaved candidate cannot survive final app/process disposal.",style=MaterialTheme.typography.caption)
            }
            TextButton({controller.discard()},enabled=state.idle){Text("Discard review and unsaved candidate")}
        }
        state.candidate?.let { candidate ->
            Divider()
            Text(if (candidate.partial) "Partial candidate" else "Provider candidate", style = MaterialTheme.typography.subtitle1)
            AiView.entries.chunked(3).forEach { row -> Row { row.forEach { mode -> TextButton({ controller.view(mode) }, enabled = state.idle) { Text((if (state.view == mode) "• " else "") + mode.name) } } } }
            if (state.view == AiView.Wipe) Slider(state.wipe, controller::wipe, enabled = state.idle)
            if (state.view == AiView.Difference) { Text("Difference gain ${state.differenceGain}×"); Slider(state.differenceGain.toFloat(), { controller.difference(it.toInt()) }, valueRange = 1f..16f, steps = 14, enabled = state.idle) }
            AiCheck("Partial-accept brush", state.brush, controller::brush, state.idle && !state.stale && !state.retrySave)
            if (state.brush) {
                Text("Radius ${state.radius.toInt()} document px")
                Slider(state.radius.toFloat(), { controller.radius(it.toDouble()) }, valueRange = .5f..256f, enabled = state.idle)
                AiCheck("Subtract acceptance", state.subtract, controller::subtract, state.idle)
                AiCheck("Start the next stroke from an empty acceptance mask", state.clearNext, controller::clearNext, state.idle)
            }
            Text("Outside changed: ${candidate.proof.changedOutside}; unaccepted changed: ${candidate.proof.changedUnaccepted ?: "not a partial mask"}", style = MaterialTheme.typography.caption)
            Text("Inside ΔE2000 mean ${candidate.proof.deltaE2000Mean}, max ${candidate.proof.deltaE2000Max}; SSIM ${candidate.proof.ssimInsideMask}", style = MaterialTheme.typography.caption)
            Text(when (candidate.settlement) { AiSettlement.UsagePriced -> "Recorded charge: ${candidate.actualMicrousd?.let(::aiMoney) ?: "unavailable"}"
                AiSettlement.UsageMissing -> "Provider usage is missing; charge remains unresolved."
                AiSettlement.LedgerUnavailable -> "Charge settlement could not be recorded. Existing history is preserved." }, style = MaterialTheme.typography.caption)
            Button({ controller.save() }, Modifier.fillMaxWidth(), enabled = state.idle && state.receipt == null && (!state.stale || state.retrySave)) {
                Text(if (state.retrySave) "Retry exact Result save" else "Save as Result layer")
            }
        }
        state.receipt?.let { receipt ->
            Text("Result saved at revision ${receipt.revision.hostSeq}. Original, response, composite and acceptance assets remain separate.")
            Row { TextButton({ controller.savedStatus(receipt.resultId, true) }, enabled = state.idle) { Text("Accept saved Result") }
                TextButton({ controller.savedStatus(receipt.resultId, false) }, enabled = state.idle) { Text("Reject / hide") } }
        }
        if (state.savedResults.isNotEmpty()) {
            Text("Saved document Results", style = MaterialTheme.typography.subtitle2)
            for (result in state.savedResults) Row(verticalAlignment = Alignment.CenterVertically) {
                Text(result.label, Modifier.weight(1f)); TextButton({ controller.savedStatus(result.resultId, true) }, enabled = enabled) { Text("Accept") }
                TextButton({ controller.savedStatus(result.resultId, false) }, enabled = enabled) { Text("Hide") }
            }
        }
        if (!state.idle) { LinearProgressIndicator(Modifier.fillMaxWidth()); Text(state.busy.name) }
        if (state.stale) Text("Document changed. A new review is required; a prior send is never repeated automatically.", color = MaterialTheme.colors.error)
        localMessage?.let { Text(it, color = MaterialTheme.colors.error) }
        state.message?.let { Text(it, style = MaterialTheme.typography.caption) }
    }
}

@Composable private fun AiSmallInput(label: String, value: String, change: (String) -> Unit, enabled: Boolean) {
    OutlinedTextField(value, change, Modifier.fillMaxWidth(), label = { Text(label) }, singleLine = true, enabled = enabled)
}
@Composable private fun AiCheck(label: String, value: Boolean, change: (Boolean) -> Unit, enabled: Boolean) {
    Row(verticalAlignment = Alignment.CenterVertically) { Checkbox(value, change, enabled = enabled); Text(label, style = MaterialTheme.typography.body2) }
}
@Composable private fun AiSettings(service: AiServiceController, state: AiServiceState, onKeys: () -> Unit, idle: Boolean) {
    val enabled=idle&&!state.busy
    val configuration = state.configuration
    var json by remember(configuration?.fingerprint) { mutableStateOf(configuration?.json.orEmpty()) }
    var dollars by remember(configuration?.fingerprint) { mutableStateOf(configuration?.dailySoftBudgetMicrousd?.let { aiMoney(it).removePrefix("$") }.orEmpty()) }
    Text("AI settings", style = MaterialTheme.typography.subtitle1)
    Text("Initialize only on first use. Missing or corrupt existing budget history is never reset.", style = MaterialTheme.typography.caption)
    Row { TextButton({ service.activate() }, enabled = enabled) { Text("Open / refresh") }
        TextButton({ service.initialize() }, enabled = enabled && configuration == null) { Text("Initialize first-use history") } }
    if (configuration != null) {
        Text("${configuration.model} · ${configuration.quality}\nVerified ${configuration.verifiedOn}; expires ${configuration.expiresOn}")
        AiSmallInput("Daily soft budget in USD", dollars, { dollars = it.take(27) }, enabled)
        val amount = aiParseMoney(dollars)
        TextButton({ amount?.let { service.configureDailyBudget(configuration.fingerprint, it) } }, enabled = enabled && amount != null) { Text("Save daily budget") }
        OutlinedTextField(json, { json = it.take(65_536) }, Modifier.fillMaxWidth(), label = { Text("Versioned provider configuration JSON") }, enabled = enabled)
        TextButton({ service.configure(json, configuration.fingerprint) }, enabled = enabled) { Text("Validate & save configuration") }
        TextButton(onKeys, enabled = enabled) { Text("Protected API key settings") }
    }
    state.readiness?.let { Text("Recorded today ${aiMoney(it.spentTodayMicrousd)} of ${aiMoney(it.dailySoftBudgetMicrousd)} soft budget.\nCredentials ${if (it.trustReady) "ready" else "unavailable"}; configuration ${if (it.configurationCurrent) "current" else "expired"}.", style = MaterialTheme.typography.caption) }
    state.failure?.let { Text(aiFailureText(it), color = MaterialTheme.colors.error) }
    if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
}
