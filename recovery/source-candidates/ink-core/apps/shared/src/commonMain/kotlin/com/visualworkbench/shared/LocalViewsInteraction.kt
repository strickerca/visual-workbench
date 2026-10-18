package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

public data class LocalViewsState(public val binding:LocalViewsBinding?=null,public val active:Boolean=false,
    public val loading:Boolean=false,public val saving:Boolean=false,public val available:Boolean=false,
    public val bookmarks:List<RegionBookmark> = emptyList(),public val retryPending:Boolean=false,public val issue:String?=null)

/** UI-thread owner of one project/document's local settings. One conflated
 * camera slot and one explicit bookmark/retry slot bound both memory and work.
 * attach/detach must be awaited before rebinding/closing the borrowed project.
 * The actor is never cancelled just because a dialog is dismissed. */
public class LocalViewsInteraction(
    private val scope:CoroutineScope,
    private val newId:()->String,
    private val factory:(WorkbenchProject)->WorkbenchLocalViews=::localViewWorkflows,
    private val debounceMs:Long=400,
) {
    private val mutable:MutableStateFlow<LocalViewsState> = MutableStateFlow(LocalViewsState())
    public val state:StateFlow<LocalViewsState> = mutable
    private val transitions:Mutex=Mutex()
    private var attached:Attachment?=null
    private data class Pending(val operation:String,val expected:ULong,val mutation:LocalViewsMutation)
    private class Attachment(val binding:LocalViewsBinding,val api:WorkbenchLocalViews,var snapshot:LocalViewsSnapshot,val epoch:Long) {
        val wake:Channel<Unit> = Channel(Channel.CONFLATED)
        lateinit var worker:Job
        var desired:Camera?=snapshot.camera
        var observed:Camera?=null
        var explicit:LocalViewsMutation?=null
        var pending:Pending?=null
        var failed:Boolean=false
        var retry:Boolean=false
        var reload:Boolean=false
        var closing:Boolean=false
    }
    init { require(debounceMs in 0..2000) }
    public fun active(value:Boolean):Unit {mutable.value=mutable.value.copy(active=value)}

    /** Returns exact stored center/scale/rotation with the real current viewport
     * dimensions. No query result is applied to a different attachment. */
    public suspend fun attach(project:WorkbenchProject,binding:LocalViewsBinding,epoch:Long,viewport:Camera):Camera? = transitions.withLock {
        stop()
        currentCoroutineContext().ensureActive()
        if(scope.coroutineContext[Job]?.isActive==false)throw CancellationException("Local view owner is closed")
        mutable.value=LocalViewsState(binding=binding,loading=true)
        try {
            val api=factory(project)
            val snapshot=api.snapshot(binding)
            validateSnapshot(binding,snapshot)
            currentCoroutineContext().ensureActive()
            val owner=Attachment(binding,api,snapshot,epoch)
            attached=owner
            owner.worker=scope.launch(start=CoroutineStart.LAZY){run(owner)}
            publish(owner)
            owner.worker.start()
            snapshot.camera?.copy(viewportWidth=viewport.viewportWidth,viewportHeight=viewport.viewportHeight)
        } catch(cancel:CancellationException) {
            mutable.value=LocalViewsState(binding=binding,issue="Local view loading was cancelled. The saved view is retained.");throw cancel
        } catch(error:Exception) {
            mutable.value=LocalViewsState(binding=binding,issue=message(error));null
        }
    }
    public fun remember(camera:Camera,epoch:Long,documentId:String):Unit {
        val owner=attached?:return
        if(owner.closing||owner.epoch!=epoch||owner.binding.documentId!=documentId||!validLocalCamera(camera))return
        if(owner.observed==camera)return
        owner.observed=camera
        owner.desired=camera
        owner.wake.trySend(Unit)
    }
    public fun addBookmark(binding:LocalViewsBinding,name:String,camera:Camera):Boolean {
        if(!validBookmarkName(name)||!validLocalCamera(camera)){issue("Use a name of at most 256 UTF-8 bytes and a valid view.");return false}
        val owner=attached?:return false
        if(owner.binding!=binding)return false
        if(owner.snapshot.bookmarks.size>=32){issue("This document already has 32 saved views. Remove one before adding another.");return false}
        return enqueue(owner,LocalViewsMutation.AddBookmark(newId(),name,camera))
    }
    public fun renameBookmark(binding:LocalViewsBinding,id:String,name:String):Boolean {
        if(!validBookmarkName(name)){issue("Use a name of at most 256 UTF-8 bytes.");return false}
        val owner=attached?:return false
        if(owner.binding!=binding)return false
        if(owner.snapshot.bookmarks.none{it.id==id})return false
        return enqueue(owner,LocalViewsMutation.RenameBookmark(id,name))
    }
    public fun deleteBookmark(binding:LocalViewsBinding,id:String):Boolean {
        val owner=attached?:return false
        if(owner.binding!=binding)return false
        if(owner.snapshot.bookmarks.none{it.id==id})return false
        return enqueue(owner,LocalViewsMutation.DeleteBookmark(id))
    }
    public fun restoreBookmark(binding:LocalViewsBinding,id:String,viewport:Camera,epoch:Long):Camera? {
        val owner=attached?:return null
        if(owner.closing||owner.failed||owner.explicit!=null||owner.pending!=null||owner.epoch!=epoch||owner.binding!=binding)return null
        return owner.snapshot.bookmarks.firstOrNull{it.id==id}?.camera?.copy(viewportWidth=viewport.viewportWidth,viewportHeight=viewport.viewportHeight)
    }
    private fun enqueue(owner:Attachment,mutation:LocalViewsMutation):Boolean {
        if(owner.closing||owner.failed||owner.explicit!=null||owner.pending?.mutation?.let{it !is LocalViewsMutation.Remember}==true)return false
        owner.explicit=mutation;publish(owner);owner.wake.trySend(Unit);return true
    }
    public fun retry(binding:LocalViewsBinding):Boolean {
        val owner=attached?:return false
        if(owner.binding!=binding||owner.closing||!owner.failed||owner.pending==null)return false
        owner.retry=true;owner.wake.trySend(Unit);return true
    }
    /** Explicitly discards the unsaved local request and rereads local settings.
     * It never discards document edits, originals or canonical undo history. */
    public fun reloadDiscardingUnsavedView(binding:LocalViewsBinding):Boolean {
        val owner=attached?:return false
        if(owner.binding!=binding||owner.closing||!owner.failed)return false
        owner.reload=true;owner.wake.trySend(Unit);return true
    }
    public suspend fun detach():Boolean = withContext(NonCancellable){transitions.withLock{stop()}}
    private suspend fun stop():Boolean = withContext(NonCancellable) {
        val owner=attached?:return@withContext true
        owner.closing=true
        mutable.value=mutable.value.copy(loading=true)
        owner.wake.close()
        owner.worker.join()
        val saved=!owner.worker.isCancelled&&!owner.failed&&owner.pending==null&&owner.explicit==null&&owner.desired==owner.snapshot.camera
        if(attached===owner){attached=null;mutable.value=LocalViewsState(issue=if(saved)null else "The last accepted local view is retained; the most recent view could not be saved.")}
        saved
    }
    private suspend fun run(owner:Attachment) {
        try {
            // Closing the channel releases this loop after all accepted work;
            // the final drain below includes the latest conflated camera.
            for(ignored in owner.wake) {
                drain(owner)
            }
            drain(owner)
        } catch(cancel:CancellationException) {
            owner.failed=true;publish(owner,"Local view saving was cancelled after native work settled.");throw cancel
        } catch(error:Exception) {
            owner.failed=true;publish(owner,message(error))
        }
    }
    private suspend fun drain(owner:Attachment) {
        if(owner.reload) {
            owner.reload=false
            try {
                val snapshot=owner.api.snapshot(owner.binding)
                validateSnapshot(owner.binding,snapshot)
                owner.snapshot=snapshot;owner.desired=snapshot.camera;owner.pending=null;owner.explicit=null;owner.failed=false;owner.retry=false
                publish(owner)
            } catch(cancel:CancellationException){throw cancel}
            catch(error:Exception){owner.failed=true;publish(owner,message(error));return}
        }
        if(owner.failed&&!owner.retry)return
        owner.retry=false
        while(true) {
            if(owner.pending==null) {
                val explicit=owner.explicit
                val mutation=if(explicit!=null) {owner.explicit=null;explicit} else {
                    if(owner.desired==null||owner.desired==owner.snapshot.camera)return
                    if(!owner.closing&&debounceMs>0)delay(debounceMs)
                    val camera=owner.desired?:return
                    if(camera==owner.snapshot.camera)return
                    LocalViewsMutation.Remember(camera)
                }
                owner.pending=Pending(newId(),owner.snapshot.revision,mutation)
            }
            val pending=checkNotNull(owner.pending)
            publish(owner)
            try {
                val next=owner.api.change(owner.binding,pending.expected,pending.operation,pending.mutation)
                validateSnapshot(owner.binding,next)
                owner.snapshot=next;owner.pending=null;owner.failed=false
                publish(owner)
            } catch(cancel:CancellationException){throw cancel}
            catch(error:Exception){owner.failed=true;publish(owner,message(error));return}
        }
    }
    private fun publish(owner:Attachment,issue:String?=null) {
        if(attached!==owner)return
        mutable.value=LocalViewsState(owner.binding,mutable.value.active,loading=owner.closing,saving=!owner.failed&&(owner.pending!=null||owner.explicit!=null),
            available=true,bookmarks=owner.snapshot.bookmarks,retryPending=owner.failed,issue=issue)
    }
    private fun issue(value:String){mutable.value=mutable.value.copy(issue=value)}
    private fun validateSnapshot(binding:LocalViewsBinding,value:LocalViewsSnapshot) {
        if(value.binding!=binding||value.bookmarks.size>32||value.camera?.let{!validLocalCamera(it)}==true||
            value.bookmarks.any{it.id.length!=36||!validBookmarkName(it.name)||!validLocalCamera(it.camera)}||
            value.bookmarks.map{it.id}.distinct().size!=value.bookmarks.size||value.bookmarks.map{it.name}.distinct().size!=value.bookmarks.size)throw WorkflowFailure(WorkflowFailureKind.Storage)
    }
    private fun message(error:Exception):String=when((error as? WorkflowFailure)?.kind){
        WorkflowFailureKind.Stale->"Saved views changed in another owner. Retry the exact request, or reload saved views and discard the unsaved view change."
        WorkflowFailureKind.Limit->"The local view or bookmark capacity is full. Existing saved views are retained."
        WorkflowFailureKind.Backpressure->"Local view storage is busy. Retry this exact view change."
        WorkflowFailureKind.Closed,WorkflowFailureKind.Cancelled->"Local view saving stopped. The last accepted saved view is retained."
        else->"Local view storage is unavailable. Saved views are retained; document edits remain independent."
    }
}
