package com.visualworkbench.android.documents

import com.visualworkbench.bindings.core.DocumentJobResult
import com.visualworkbench.shared.*
import java.io.*

internal object DocumentWire {
    const val JOB=1; const val CANCEL=2; const val FINISHED=3
    private val MAGIC=byteArrayOf(86,87,68,65,80,48,49,0)
    fun write(output: OutputStream, result: DocumentJobResult?, failure: String?) {
        val data=DataOutputStream(output);data.write(MAGIC)
        if(result==null) {data.writeInt(1);text(data,failure ?: "Worker",64)}
        else {data.writeInt(0);text(data,result.sourceBlake3,64);text(data,result.outputBlake3,64);text(data,result.metadataJson,1024*1024);data.writeInt(result.bytes.size);data.write(result.bytes)}
        data.flush()
    }
    fun read(input: InputStream): DocumentJobResult {
        val data=DataInputStream(input);val magic=ByteArray(8);data.readFully(magic);if(!magic.contentEquals(MAGIC)) throw DocumentFailure(DocumentFailureKind.Verification)
        if(data.readInt()!=0) { val code=text(data,64);throw DocumentFailure(DocumentFailureKind.entries.firstOrNull{it.name==code} ?: DocumentFailureKind.Worker) }
        val source=text(data,64);val hash=text(data,64);val metadata=text(data,1024*1024);val length=data.readInt()
        if(length !in 0..128*1024*1024) throw DocumentFailure(DocumentFailureKind.Limit)
        val bytes=ByteArray(length);data.readFully(bytes);if(data.read()!=-1) throw DocumentFailure(DocumentFailureKind.Verification)
        return DocumentJobResult(source,metadata,bytes,hash)
    }
    fun source(input: InputStream): ByteArray {
        val output=ByteArrayOutputStream();val buffer=ByteArray(65536);var total=0
        while(true) {val count=input.read(buffer);if(count<0) break;if(count==0) throw DocumentFailure(DocumentFailureKind.Verification);total=Math.addExact(total,count);if(total>64*1024*1024) throw DocumentFailure(DocumentFailureKind.Limit);output.write(buffer,0,count)}
        if(total==0) throw DocumentFailure(DocumentFailureKind.Invalid);return output.toByteArray()
    }
    private fun text(output: DataOutputStream, value: String, cap: Int) {val bytes=value.toByteArray(Charsets.UTF_8);if(bytes.size>cap) throw DocumentFailure(DocumentFailureKind.Limit);output.writeInt(bytes.size);output.write(bytes)}
    private fun text(input: DataInputStream, cap: Int): String {val length=input.readInt();if(length !in 0..cap) throw DocumentFailure(DocumentFailureKind.Limit);val bytes=ByteArray(length);input.readFully(bytes);val result=bytes.toString(Charsets.UTF_8);if(!result.toByteArray(Charsets.UTF_8).contentEquals(bytes)) throw DocumentFailure(DocumentFailureKind.Verification);return result}
}
