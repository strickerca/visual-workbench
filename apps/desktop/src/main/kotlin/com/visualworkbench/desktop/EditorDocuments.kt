package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import com.visualworkbench.desktop.documents.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.nio.file.Path

/** Root app owner; worker/IO/cache leaves retain their own execution resources. */
internal class EditorDocuments(private val scope:CoroutineScope, private val core:WorkbenchCore,
    private val attachment:()->DocumentAttachment?, admitted:()->Boolean,
    changePage:suspend(DocumentAttachment,UInt)->Unit, private val changed:()->Unit,
    private val local:Path,
) {
    val jobs=DocumentJobsOwner()
    val controller=DocumentController(scope,core,jobs,attachment,admitted,changePage)
    private val cache=DocumentTileCache(128L*1024*1024)
    private var cacheBytes=0L
    private val gate=Mutex()
    private val retired=mutableListOf<DocumentDrawFrame<PreparedDocumentTile>>()
    var frame:DocumentDrawFrame<PreparedDocumentTile>?=null;private set
    private var observed:DocumentDisplayFrame?=null

    init {
        scope.launch {
            var initial=true
            controller.state.collect { state ->
                if(initial){initial=false;if(state.descriptor==null)return@collect}
                val source=state.frame
                if(source===observed){changed();return@collect}
                observed=source
                gate.withLock {
                    if(source==null){adopt(null);return@withLock}
                    try {

                        for(value in retired){value.close();value.awaitRetired()}
                        retired.clear();cache.close();cacheBytes=0
                        val requested=source.tiles.sumOf{it.rgba.size.toLong()*4+4096}
                        if(requested>128L*1024*1024)throw DocumentFailure(DocumentFailureKind.Limit)
                        cacheBytes=requested
                        val tiles=cache.prepareOwned(source){controller.state.value.frame===source&&attachment()?.let{it.epoch==source.binding.attachmentEpoch&&it.snapshot.render.revision.sameDocumentRevision(source.binding.revision)}==true}
                        var acquired:DocumentDrawFrame<PreparedDocumentTile>?=DocumentDrawFrame(source.binding,tiles)
                        try {
                            if(controller.state.value.frame!==source)throw DocumentFailure(DocumentFailureKind.Stale)
                            adopt(acquired);acquired=null
                        }finally{acquired?.close()}
                    }catch(cancel:CancellationException){throw cancel}
                    catch(error:DocumentFailure){if(controller.state.value.frame===source)controller.reportFailure(error)}
                    catch(_:Exception){if(controller.state.value.frame===source)controller.reportFailure(DocumentFailure(DocumentFailureKind.Worker))}
                }
            }
        }
    }
    private fun adopt(next:DocumentDrawFrame<PreparedDocumentTile>?) {
        val previous=frame
        if(previous!=null){
            retired+=previous
            try{previous.close()}catch(error:Exception){frame=null;changed();throw error}
        }
        frame=next;changed()
        retired.removeAll{it.retired}
    }
    suspend fun ensureJobs() { DocumentRetirementRegistry.drain();if(!jobs.held) { jobs.install(BudgetedDocumentJobs(DesktopDocumentRuntimeCache.prepare(local),{cacheBytes})) } }
    suspend fun bind(project:WorkbenchProject,snapshot:DocumentSnapshot) {
        val native=project as? WorkbenchDocumentsProject
        val descriptor=native?.documentDescriptor(snapshot.documentId)
        if(attachment()?.project!==project)return
        controller.bind(descriptor,native?.selectedDocumentPage(snapshot.documentId)?:0u)
        if(descriptor!=null)try{ensureJobs();controller.scheduleNotices()}catch(error:DocumentFailure){controller.reportFailure(error)}
    }
    fun matchingFrame(snapshot:DocumentSnapshot,epoch:Long):DocumentDrawFrame<PreparedDocumentTile>? = frame?.takeIf {
        it.available&&it.binding==controller.state.value.frame?.binding&&it.binding.documentId==snapshot.documentId&&it.binding.page==controller.state.value.page&&it.binding.attachmentEpoch==epoch&&it.binding.revision.sameDocumentRevision(snapshot.render.revision)
    }
    suspend fun discardDraw() {
        gate.withLock {
            adopt(null)

            for(value in retired){value.close();value.awaitRetired()}
            retired.clear();cache.close();cacheBytes=0
        }
    }
    suspend fun detach() {
        controller.close()
        gate.withLock {
            observed=null;adopt(null)
            // Compose releases its independent reader after the frame leaves composition.
            for(value in retired){value.close();value.awaitRetired()}
            retired.clear();cache.close();cacheBytes=0
        }
    }
    suspend fun close() {
        withContext(NonCancellable){
            var failure:Exception?=null
            try{detach()}catch(error:Exception){failure=error}

            try{jobs.close()}catch(error:Exception){failure=error}
            try{DesktopDocumentRuntimeCache.closePending()}catch(error:Exception){failure=error}
            failure?.let{throw it}
        }
    }
}
