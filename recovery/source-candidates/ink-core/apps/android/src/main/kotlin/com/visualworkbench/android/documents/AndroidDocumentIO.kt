package com.visualworkbench.android.documents

import android.content.Context
import android.net.Uri
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.io.File

internal class AndroidDocumentIO(private val context:Context,private val core:WorkbenchCore) {
    suspend fun import(uri:Uri,kind:EditableKind,path:String,device:String,title:String,password:String?=null,progress:(String)->Unit):WorkbenchProject {
        val bytes=withContext(Dispatchers.IO) {context.contentResolver.openInputStream(uri)?.use{input ->
            val cap=if(kind==EditableKind.Svg)4*1024*1024 else 64*1024*1024
            val output=java.io.ByteArrayOutputStream();val buffer=ByteArray(65536);var total=0
            while(true){ensureActive();val count=input.read(buffer);if(count<0)break;if(count==0)throw DocumentFailure(DocumentFailureKind.Verification);total=Math.addExact(total,count);if(total>cap)throw DocumentFailure(DocumentFailureKind.Limit);output.write(buffer,0,count)}
            output.toByteArray()
        } ?: throw DocumentFailure(DocumentFailureKind.Invalid)}
        val jobs=installedAndroidDocumentJobs()
        val inspection=DocumentInteraction(jobs,core).inspect(bytes,kind,password){done,total->progress("Reading document pages $done of $total")}
        val now=System.currentTimeMillis();val id=core.newId(now.toULong())
        return (core as? WorkbenchDocumentsCore ?: throw DocumentFailure(DocumentFailureKind.Unavailable)).createDocument(CreateDocument(path,id,core.newId(now.toULong()),device,title.take(256),bytes,inspection.first.sourceBlake3,kind,inspection.second,now,password))
    }
    suspend fun publish(uri:Uri,copy:DocumentExport) = withContext(Dispatchers.IO) {
        // SAF creates a user-selected new document. Its provider owns publication
        // guarantees; a failed/partial provider write never becomes a ready copy.
        context.contentResolver.openOutputStream(uri,"w")?.use{output ->
            var offset=0
            while(offset<copy.bytes.size){ensureActive();val count=minOf(65536,copy.bytes.size-offset);output.write(copy.bytes,offset,count);offset+=count}
            output.flush()
        } ?: throw DocumentFailure(DocumentFailureKind.Worker)
    }
}
@Volatile private var androidJobs:AndroidDocumentJobs?=null
internal fun prepareAndroidDocumentJobs(context:Context) {if(androidJobs==null)synchronized(AndroidDocumentJobs::class.java){if(androidJobs==null){val jobs=AndroidDocumentJobs(context);installDocumentJobs(jobs);androidJobs=jobs}}}
internal fun installedAndroidDocumentJobs():DocumentJobs = androidJobs ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
