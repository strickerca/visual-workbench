package com.visualworkbench.android.documents

import com.visualworkbench.shared.DocumentFailure
import com.visualworkbench.shared.DocumentFailureKind
import java.io.InputStream

/** API-29-compatible bounded read. One extra byte detects an oversized pin;
 * no zero-progress loop, unbounded read or newer InputStream API is used. */
internal fun readDocumentRuntimePin(input:InputStream):ByteArray {
    val bytes=ByteArray(1025);var used=0
    while(used<bytes.size){
        val count=input.read(bytes,used,bytes.size-used)
        if(count<0)break
        if(count==0||count>bytes.size-used)throw DocumentFailure(DocumentFailureKind.Verification)
        used+=count
    }
    if(used !in 1..1024)throw DocumentFailure(DocumentFailureKind.Verification)
    return bytes.copyOf(used)
}
