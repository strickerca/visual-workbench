package com.visualworkbench.desktop.remote

import androidx.compose.material.Text
import androidx.compose.material.TextButton
import androidx.compose.runtime.*
import com.visualworkbench.shared.*
import java.awt.EventQueue
import java.awt.FileDialog
import java.awt.Frame
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.nio.file.attribute.BasicFileAttributes
import kotlinx.coroutines.*
import kotlin.coroutines.resume

/** Real palette entry point. The selected file is untrusted input: only the
 * current observation owner's reader can admit it. No path or receipt is saved. */
@Composable internal fun RemoteCompatibilityImportButton(palette:RemoteEditorPalette) {
    val state by palette.state.collectAsState()
    val scope=rememberCoroutineScope()
    var selecting by remember{mutableStateOf(false)}
    var message by remember{mutableStateOf<String?>(null)}
    TextButton(onClick={
        if(selecting)return@TextButton
        val expected=palette.state.value
        selecting=true;message=null
        scope.launch {
            try {
                val path=chooseCompatibilityReceipt()?:return@launch
                if(!sameImportTarget(expected,palette.state.value)){message="The selected editor changed. No receipt was read.";return@launch}
                val bytes=withContext(Dispatchers.IO){readCompatibilityReceipt(path)}
                try {
                    if(!sameImportTarget(expected,palette.state.value)||
                        !palette.importCompatibilityReceipt(bytes,expected.target))message="Compatibility import was not started. Recheck the selected editor."
                }finally{bytes.fill(0)}
            }catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){message="The selected receipt could not be read within its size and file limits."}
            finally{selecting=false}
        }
    },enabled=!selecting&&!state.closed&&!state.busy&&state.target?.identity?.editor==RemoteEditorKind.Paint){Text("Import compatibility observation receipt")}
    Text("Receipt files cannot grant control or prove support by themselves.")
    message?.let{Text(it)}
}
internal fun sameImportTarget(before:RemoteEditorPaletteState,after:RemoteEditorPaletteState):Boolean =
    !before.closed&&!after.closed&&before.target!=null&&before.target==after.target&&before.targetRevision==after.targetRevision

internal fun readCompatibilityReceipt(path:Path):ByteArray {
    val limit=32*1024
    val info=Files.readAttributes(path,BasicFileAttributes::class.java,LinkOption.NOFOLLOW_LINKS)
    require(info.isRegularFile&&!info.isSymbolicLink&&info.size() in 1..limit.toLong())
    return FileChannel.open(path,StandardOpenOption.READ,LinkOption.NOFOLLOW_LINKS).use{channel->
        val buffer=ByteBuffer.allocate(limit+1)
        while(buffer.hasRemaining()){
            val count=channel.read(buffer)
            if(count<0)break
            require(count>0)
        }
        require(buffer.position() in 1..limit)
        buffer.flip();ByteArray(buffer.remaining()).also{buffer.get(it)}
    }
}
private suspend fun chooseCompatibilityReceipt():Path? = suspendCancellableCoroutine{continuation->
    val dialog=java.util.concurrent.atomic.AtomicReference<FileDialog?>()
    continuation.invokeOnCancellation{EventQueue.invokeLater{dialog.getAndSet(null)?.dispose()}}
    EventQueue.invokeLater {
        if(!continuation.isActive)return@invokeLater
        val chooser=try{FileDialog(null as Frame?,"Import owned Paint observation receipt",FileDialog.LOAD)}
            catch(error:Exception){if(continuation.isActive)continuation.resumeWith(Result.failure(error));return@invokeLater}
        dialog.set(chooser)
        try {
            if(!continuation.isActive)return@invokeLater
            chooser.isMultipleMode=false
            chooser.isVisible=true
            val name=chooser.file
            val selected=if(name==null)null else Path.of(chooser.directory,name)
            if(continuation.isActive)continuation.resume(selected)
        }catch(error:Exception){if(continuation.isActive)continuation.resumeWith(Result.failure(error))}
        finally{dialog.compareAndSet(chooser,null);chooser.dispose()}
    }
}
