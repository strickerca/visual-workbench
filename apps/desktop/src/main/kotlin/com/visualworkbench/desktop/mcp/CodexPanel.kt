package com.visualworkbench.desktop.mcp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.PublishedPackage
import kotlinx.coroutines.*
import java.nio.file.Path

/** Owner-only panel: never constructed from an MCP request or a startup marker.
 * No handler selects a thread, stages images or sends a turn implicitly. */
@Composable internal fun CodexPanel(owner:DesktopCodexHandoff,packages:List<PublishedPackage>,
    chooseExecutable:()->Path?,onClose:()->Unit) {
    val state by owner.state.collectAsState();val scope=rememberCoroutineScope()
    var executable by remember{mutableStateOf<Path?>(null)}
    var acknowledgment by remember(state.preview?.previewId){mutableStateOf(false)}
    var chooserIssue by remember{mutableStateOf<String?>(null)}
    fun act(body:suspend()->Unit){scope.launch(start=CoroutineStart.UNDISPATCHED){
        try{body()}catch(error:CancellationException){throw error}catch(_:Exception){/* Typed owner state owns the report; never log private output. */}
    }}
    AlertDialog(onDismissRequest=onClose,title={Text("Send a package to Codex")},text={
        Column(Modifier.width(850.dp).heightIn(max=740.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)){
            Text("Choose the installed CLI explicitly. Workbench starts a private App Server, checks its generated schema, and uses its existing account and thread settings. No approval request is accepted by this adapter.")
            Text("This controls the selected runtime's existing thread. It does not configure a desktop Codex client or create a new thread.",style=MaterialTheme.typography.caption)
            Text(executable?.toString()?:"No executable selected",style=MaterialTheme.typography.caption)
            Row{
                TextButton(enabled=!state.ready&&!state.busy&&!state.closed,onClick={try{executable=chooseExecutable()?:executable;chooserIssue=null}catch(_:Exception){chooserIssue="The executable chooser was unavailable; no runtime was started."}}){Text("Choose codex.exe")}
                Button(enabled=executable!=null&&!state.ready&&!state.busy&&!state.closed,onClick={val exact=executable;if(exact!=null)act{owner.open(exact)}}){Text("Check selected runtime")}
            }
            if(state.ready){
                Divider();Text("Existing threads",style=MaterialTheme.typography.h6)
                Row{
                    TextButton(enabled=!state.busy,onClick={act{owner.list()}}){Text("Refresh first page")}
                    val cursor=state.cursor
                    TextButton(enabled=!state.busy&&cursor!=null,onClick={if(cursor!=null)act{owner.list(cursor)}}){Text("Next page")}
                }
                state.threads.forEach{thread->Column(Modifier.border(1.dp,MaterialTheme.colors.onSurface.copy(alpha=.2f)).padding(8.dp)){
                    Text(thread.preview.ifBlank{"Untitled thread"},maxLines=3)
                    Text("${thread.id} · ${thread.status}",style=MaterialTheme.typography.caption)
                    TextButton(enabled=!state.busy&&thread.status in listOf("idle","notLoaded"),onClick={act{owner.select(thread)}}){Text("Select this exact thread")}
                }}
                state.selected?.let{selected->
                    Divider();Text("Selected: ${selected.threadId}");Text("Model: ${selected.model}")
                    Text(selected.cwd,style=MaterialTheme.typography.caption)
                    Text("Immutable packages",style=MaterialTheme.typography.h6)
                    if(packages.none{it.info.target in listOf("generic","openai")})Text("Use Local agents and Compare to explicitly start the package owner and compile a generic or OpenAI package first. No second catalog owner is opened here.")
                    packages.filter{it.info.target in listOf("generic","openai")}.forEach{saved->
                        Text("${saved.info.target} · ${saved.info.packageId} · revision ${saved.info.binding.hostSeq}")
                        TextButton(enabled=!state.busy,onClick={act{owner.preview(saved)}}){Text("Prepare exact Send preview")}
                    }
                }
            }
            state.preview?.let{displayed->
                Divider();Text("Review before Send",style=MaterialTheme.typography.h6)
                Text("Thread ${displayed.threadId} · model ${displayed.model}")
                Text("Package ${displayed.packageId} · ${displayed.manifestSha256}",style=MaterialTheme.typography.caption)
                Text("Runtime ${displayed.binarySha256} · schema ${displayed.schemaSha256}",style=MaterialTheme.typography.caption)
                Text(displayed.text)
                displayed.images.forEachIndexed{index,image->Text("Image ${index+1}: ${image.width} × ${image.height} px · ${image.sha256}",style=MaterialTheme.typography.caption)}
                Text("Image detail: ${displayed.detail}. The selected runtime's verified preprocessing profile admits these exact dimensions. This panel lists the immutable image inventory; inspect the compiled package images before confirming.")
                Row{Checkbox(acknowledgment,{acknowledgment=it},enabled=!state.busy);Text("I reviewed this package and the exact thread, text and image inventory above.")}
                Button(enabled=acknowledgment&&!state.busy,onClick={val accepted=displayed;if(acknowledgment)act{owner.send(accepted)}}){Text("Send once to this Codex thread")}
            }
            if(state.busy)LinearProgressIndicator(Modifier.fillMaxWidth())
            Text(chooserIssue?:state.status,color=MaterialTheme.colors.secondary)
            Text("No automatic resend. A failed or cancelled Send may already have reached Codex; inspect the selected thread. Stopping this private App Server may end its active tool work. Private input copies remain for explicit recovery.",style=MaterialTheme.typography.caption)
            Row{
                TextButton(enabled=state.ready&&!state.busy,onClick={act{owner.interrupt()}}){Text("Interrupt known turn")}
                TextButton(enabled=!state.closed,onClick={act{owner.close()}}){Text("Stop private App Server")}
            }
            if(state.closed)Text("This owner is closed. Restart Workbench before opening another private App Server.")
        }
    },confirmButton={TextButton(onClick=onClose){Text("Close panel")}})
}
