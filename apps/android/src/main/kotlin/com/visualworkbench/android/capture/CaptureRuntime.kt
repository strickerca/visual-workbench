package com.visualworkbench.android.capture

import android.content.Context
import com.visualworkbench.shared.*
import java.io.File
import java.io.FileOutputStream
import java.util.UUID
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Installed by the initialized app repository. No fallback device identity,
 * hidden store, or capture action is created by service startup. */
internal class CaptureRuntime(
    val core:WorkbenchCore,val deviceId:String,
    val projectPath:(String)->String,val lamport:()->ULong,
    val accepted:suspend (WorkbenchProject,String?)->Unit,
) {
    private val closed=java.util.concurrent.atomic.AtomicBoolean()
    companion object { val installed=AtomicReference<CaptureRuntime?>(null) }
    fun close(){closed.set(true);installed.compareAndSet(this,null)}
    suspend fun importAndCollect(lease:CaptureLease,frame:CaptureFrame,collect:suspend()->CapturedTree) {
        check(!closed.get())
        val now=System.currentTimeMillis();fun id()=core.newId(now.toULong())
        val projectId=id();val documentId=id();var project:WorkbenchProject?=null;var adopted=false
        try{
            project=createCaptureProject(CreateFileProject(projectPath(projectId),projectId,documentId,id(),deviceId,"Screen capture",lease.png.absolutePath,lease.folder.absolutePath,now),
                CaptureImportDescriptor(true,id(),1uL,1u,"android","monitor",0,0,frame.width.toUInt(),frame.height.toUInt(),1.0,
                    frame.timestampMs.toULong()*1_000_000uL,frame.capturedAtMs))
            check(!closed.get())
            // The exact screenshot now exists canonically. A private ticket is
            // created BEFORE any tree collection, so a late tree cannot rebind.
            val info=project.info();val binding=WorkflowBinding(info.projectId,documentId,info.hostSeq,info.stateHash)
            val ticket=semanticWorkflows(project).beginCapture(binding)
            var warning:String?=null
            try{
                val tree=collect()
                val plan=ticket.prepare(WorkflowMetadata(id(),deviceId,lamport(),System.currentTimeMillis()),id(),SemanticPlatform.AndroidAx,
                    CaptureAdmission.delta(frame.timestampMs,tree.observedMs),tree.elapsedMs.toULong(),SemanticBoundsSpace.CapturePixels,tree.elements)
                try{plan.commit()}finally{plan.close()}
            }catch(error:kotlinx.coroutines.CancellationException){throw error}
            catch(_:Exception){warning="Screenshot saved; accessibility information was unavailable or the source changed."}
            finally{ticket.close()}
            // Native import has durably preserved the original. Cleanup only
            // touches the exact private lease; publication never depends on it.
            lease.release()
            check(!closed.get());accepted(project,warning);adopted=true
        }finally{if(!adopted)withContext(NonCancellable){project?.close()}}
    }
}
internal class CaptureLease private constructor(val folder:File,val png:File,private val token:String) {
    fun release(){
        val marker=File(folder,"owner-v1")
        require(folder.canonicalFile==folder.absoluteFile&&marker.isFile&&marker.length()==token.length.toLong()&&marker.readText()==token)
        require(folder.listFiles()?.all{it.name in setOf("owner-v1","capture.png")}==true)
        if(png.exists())check(png.delete());check(marker.delete());check(folder.delete())
    }
    companion object {
        fun retained(context:Context):List<CaptureLease>{
            val root=File(context.filesDir,"capture-inbox")
            if(!root.exists())return emptyList()
            require(root.canonicalFile==root.absoluteFile)
            return (root.listFiles()?:error("Capture inbox unavailable")).take(64).mapNotNull{runCatching{open(context,it.name)}.getOrNull()}.filter{it.png.length()>0}
        }
        fun open(context:Context,name:String):CaptureLease{
            require(UUID.fromString(name).toString()==name)
            val root=File(context.filesDir,"capture-inbox");val folder=File(root,name)
            require(root.canonicalFile==root.absoluteFile&&folder.canonicalFile==folder.absoluteFile&&folder.isDirectory)
            val marker=File(folder,"owner-v1");require(marker.isFile&&marker.canonicalFile==marker.absoluteFile&&marker.length()==36L)
            val token=marker.readText();require(UUID.fromString(token).toString()==token)
            val png=File(folder,"capture.png");require(png.canonicalFile==png.absoluteFile&&png.isFile&&png.length()<=CaptureAdmission.ENCODED)
            return CaptureLease(folder,png,token)
        }
        fun create(context:Context):CaptureLease{
            val root=File(context.filesDir,"capture-inbox");check(root.isDirectory||root.mkdir());require(root.canonicalFile==root.absoluteFile)
            val entries=root.listFiles()?:error("Capture inbox unavailable");require(entries.size<64)
            val used=entries.sumOf{entry->entry.listFiles()?.sumOf{if(it.isFile)it.length() else CaptureAdmission.ENCODED}?:CaptureAdmission.ENCODED}
            require(used<=1024L*1024*1024-CaptureAdmission.ENCODED)
            val folder=File(root,UUID.randomUUID().toString());check(folder.mkdir());val token=UUID.randomUUID().toString()
            try{FileOutputStream(File(folder,"owner-v1")).use{it.write(token.toByteArray());it.fd.sync()};return CaptureLease(folder,File(folder,"capture.png"),token)}
            catch(error:Throwable){File(folder,"owner-v1").delete();folder.delete();throw error}
        }
    }
}
internal data class CaptureFrame(val width:Int,val height:Int,val timestampMs:Long,val capturedAtMs:Long)
