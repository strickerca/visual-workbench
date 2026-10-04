package com.visualworkbench.desktop.mcp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import com.visualworkbench.bindings.host.excludeCaptureWindows
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

internal data class McpCompileChoice(val target:PackageTarget,val semanticSnapshotId:String?,val includeWindowTitle:Boolean,val assumeSrgb:Boolean,val allowDepth:Boolean)

/** Explicit local owner controls. Bridge PIDs identify an authenticated local
 * connection, not an asserted product identity. No connection is auto-granted. */
@Composable internal fun McpSettingsPanel(owner:DesktopMcpCoordinator,onCompile:suspend(McpCompileChoice)->Unit,onCompare:(McpInboxReceipt)->Unit,onClose:()->Unit,onStop:()->Unit={}) {
    val state by owner.state.collectAsState();val scope=rememberCoroutineScope()
    var busy by remember{mutableStateOf(false)};var message by remember{mutableStateOf<String?>(null)}
    var selectedAgent by remember{mutableStateOf<String?>(null)};var selectedTarget by remember{mutableStateOf<String?>(null)}
    var profile by remember{mutableStateOf("generic")};var model by remember{mutableStateOf("")}
    var snapshot by remember{mutableStateOf("")};var title by remember{mutableStateOf(false)}
    var assumeSrgb by remember{mutableStateOf(false)};var depth by remember{mutableStateOf(false)}
    fun act(body:suspend()->Unit){if(busy)return;busy=true;message=null;scope.launch{try{body()}catch(error:CancellationException){throw error}catch(_:Exception){message="The operation was refused or did not finish. Refresh to recover durable publication; do not assume it was sent."}finally{busy=false}}}
    AlertDialog(onDismissRequest=onClose,title={Text("Local agent access")},text={Column(Modifier.width(850.dp).heightIn(max=750.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)){
        Text("Local service: 127.0.0.1:${state.port} | credentials stay in Windows Credential Manager")
        Text("Access is limited to immutable packages. Capture requires a separate grant for a selected window and connection.",style=MaterialTheme.typography.caption)
        Text("Connected bridges",style=MaterialTheme.typography.h6)
        if(state.agents.isEmpty())Text("No local bridge is connected. This screen does not configure or start an external client.")
        state.agents.forEach{agent->Row{RadioButton(selectedAgent==agent.connection,{selectedAgent=agent.connection});Text("Bridge process ${agent.bridgePid} | ${agent.connection.take(8)}",Modifier.padding(top=12.dp))}}
        Divider();Text("Compile this document",style=MaterialTheme.typography.h6)
        Row{listOf("generic","claude","claude-legacy","openai","gemini").forEach{name->TextButton(onClick={profile=name},enabled=!busy){Text(if(name==profile)"[$name]"else name)}}}
        if(profile!="generic")OutlinedTextField(model,{model=it.take(256)},label={Text("Explicit target model")},enabled=!busy)
        OutlinedTextField(snapshot,{snapshot=it.take(36)},label={Text("Semantic snapshot UUID | optional, no automatic capture")},enabled=!busy)
        Row{Checkbox(title,{title=it});Text("Include the captured window title")}
        Row{Checkbox(assumeSrgb,{assumeSrgb=it});Text("Treat untagged source colors as sRGB for package images")}
        Row{Checkbox(depth,{depth=it});Text("Allow 16-to-8-bit package image conversion; preserve project original")}
        Button(enabled=!busy&&(profile=="generic"||model.isNotBlank()),onClick={
            val target=when(profile){"claude"->PackageTarget.ClaudeModern(model);"claude-legacy"->PackageTarget.ClaudeLegacy(model);"openai"->PackageTarget.OpenAiResponses(model,2048u);"gemini"->PackageTarget.Gemini(model,2048u);else->PackageTarget.Generic()}
            val choice=McpCompileChoice(target,snapshot.takeIf{it.isNotBlank()},title,assumeSrgb,depth)
            act{onCompile(choice);owner.refresh()}
        }){Text("Compile and expose immutable package")}
        Text("Compilation does not send a message. A target profile does not prove that the connected client supports inline images.",style=MaterialTheme.typography.caption)
        state.packages.forEach{published->val info=published.info
            Column(Modifier.border(1.dp,MaterialTheme.colors.onSurface.copy(alpha=.2f)).padding(8.dp)){
                Text("${info.target} | ${info.markerCount} markers | revision ${info.binding.hostSeq}")
                Text(info.manifestSha256,style=MaterialTheme.typography.caption)
                Row{
                    TextButton(enabled=!busy,onClick={act{owner.expose(published)}}){Text("Expose this saved package")}
                    if(info.target=="claude")TextButton(enabled=!busy,onClick={act{owner.previewClaude(published)}}){Text("Review Claude Send")}
                    TextButton(enabled=!busy,onClick={act{owner.retirePackage(published)}}){Text("Unpublish and retire")}
                }
            }
        }
        state.sendPreview?.let{displayed->
            Divider();Text("Review exact Send payload",style=MaterialTheme.typography.h6)
            Text(displayed.content);Text(displayed.folder,style=MaterialTheme.typography.caption)
            Text(displayed.manifestSha256,style=MaterialTheme.typography.caption)
            val connection=selectedAgent
            Button(enabled=!busy&&state.agents.any{it.connection==connection},onClick={if(connection!=null)act{owner.sendClaude(connection,displayed)}}){Text("Send once to selected bridge")}
            Text("Only the transport write can be confirmed. The external agent may decline or ignore it.",style=MaterialTheme.typography.caption)
        }
        Text("Use Workbench → Send package to Codex for explicit runtime and existing-thread selection. Send remains disabled without the exact installed schema and a verified image preprocessing profile.",style=MaterialTheme.typography.caption)
        Divider();Text("Capture grants",style=MaterialTheme.typography.h6)
        Text("Select target: activate the intended window, then press Ctrl+Alt+M within 15 seconds. The exact window/process identity is retained; foreground changes never substitute another target.")
        Button(enabled=!busy&&state.targets.size<16,onClick={act{owner.selectTarget()}}){Text("Choose a capture window")}
        state.targets.forEach{target->Row{RadioButton(selectedTarget==target.selector,{selectedTarget=target.selector});Text(target.description,Modifier.padding(top=12.dp))}}
        val grantAgent=selectedAgent;val grantTarget=selectedTarget
        Button(enabled=!busy&&state.agents.any{it.connection==grantAgent}&&grantTarget!=null,onClick={if(grantAgent!=null&&grantTarget!=null)act{owner.grant(grantAgent,listOf(grantTarget),60_000)}}){Text("Grant selected bridge 60 seconds for this window")}
        state.grants.forEach{grant->Row{Text("${grant.connection.take(8)} | ${grant.selectors.size} window(s)",Modifier.weight(1f));TextButton(onClick={act{owner.revoke(grant.connection)}},enabled=!busy){Text("Revoke")}}}
        Text("Grants expire, are lost on restart/disconnect, and never permit capture of app-owned windows. A visible capture indicator must draw before every request.",style=MaterialTheme.typography.caption)
        Divider();Text("Compare inbox",style=MaterialTheme.typography.h6)
        state.inbox.filterNot{it.retired}.forEach{receipt->Row{
            Text("${if(receipt.after!=null)"PNG"else"Text"} | revision ${receipt.binding.hostSeq} | ${receipt.receiptId.take(8)}",Modifier.weight(1f))
            TextButton(onClick={onCompare(receipt)},enabled=!busy){Text("Compare")}
            TextButton(onClick={act{owner.retireReturn(receipt)}},enabled=!busy){Text("Retire")}
        }}
        if(busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        (message?:state.message)?.let{Text(it,color=MaterialTheme.colors.secondary)}
    }},confirmButton={TextButton(onClick={act{owner.refresh()}},enabled=!busy){Text("Refresh")}},dismissButton={Row{TextButton(onClick=onStop){Text("Stop service and revoke all")};TextButton(onClick=onClose){Text("Close")}}})
}

/** This independent always-on-top window remains visible when the editor is
 * minimized. The native affinity refusal prevents acknowledgment and capture. */
@Composable internal fun McpCaptureIndicator(owner:DesktopMcpCoordinator) {
    val state by owner.state.collectAsState();val activity=state.activeCapture
    if(!mcpGrantIndicatorVisible(state))return
    val scope=rememberCoroutineScope()
    var stopping by remember(owner){mutableStateOf(false)}
    var issue by remember(owner){mutableStateOf<String?>(null)}
    fun revoke(){if(stopping)return;stopping=true;scope.launch{try{
        owner.revokeAll()
    }catch(error:CancellationException){throw error}
    catch(_:Exception){issue="Local revocation was requested. Cleanup is uncertain; retry or stop the service."}
    finally{stopping=false}}}
    Window(onCloseRequest={revoke()},title="Visual Workbench capture grants",alwaysOnTop=true,resizable=false,
        state=rememberWindowState(width=410.dp,height=170.dp)) {
        LaunchedEffect(window,activity?.id){
            try {
                excludeCaptureWindows(listOf(window.windowHandle.toULong()))
                withFrameNanos{};withFrameNanos{}
                if(window.isShowing&&activity!=null)owner.indicatorDrawn(activity.id)
            } catch(error:CancellationException){throw error}
            catch(_:Exception){/* No draw acknowledgment: the actual capture gate refuses. */}
        }
        MaterialTheme {Surface(Modifier.fillMaxSize()){Column(Modifier.padding(16.dp),verticalArrangement=Arrangement.spacedBy(8.dp)){
            Text(if(activity!=null)"Capturing the granted window"else"Agent capture permission is enabled",style=MaterialTheme.typography.h6)
            Text("${state.grants.size} active grant(s). This indicator remains until grants end and capture work settles.",style=MaterialTheme.typography.caption)
            Button(onClick={revoke()},enabled=!stopping){Text("Revoke capture grants")}
            issue?.let{Text(it,style=MaterialTheme.typography.caption)}
        }}}
    }
}
