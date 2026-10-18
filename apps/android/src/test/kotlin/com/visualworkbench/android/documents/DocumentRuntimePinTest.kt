package com.visualworkbench.android.documents

import com.visualworkbench.shared.DocumentFailure
import com.visualworkbench.shared.DocumentFailureKind
import java.io.ByteArrayInputStream
import org.junit.Assert.*
import org.junit.Test

class DocumentRuntimePinTest {
    @Test fun shortReadsPreserveEveryAdmittedByteAndStopAtEof(){
        val bytes=ByteArray(1024){(it%127).toByte()}
        val input=object:ByteArrayInputStream(bytes){override fun read(buffer:ByteArray,offset:Int,length:Int):Int=super.read(buffer,offset,minOf(3,length))}
        assertArrayEquals(bytes,readDocumentRuntimePin(input))
    }
    @Test fun oversizeIsRefusedAfterOnlyOneExtraByte(){
        val input=ByteArrayInputStream(ByteArray(4096){65})
        assertEquals(DocumentFailureKind.Verification,assertThrows(DocumentFailure::class.java){readDocumentRuntimePin(input)}.kind)
        assertEquals(4096-1025,input.available())
    }
    @Test fun emptyAndZeroProgressCannotBeAdmittedOrSpin(){
        assertThrows(DocumentFailure::class.java){readDocumentRuntimePin(ByteArrayInputStream(byteArrayOf()))}
        val input=object:ByteArrayInputStream(byteArrayOf(1)){override fun read(buffer:ByteArray,offset:Int,length:Int):Int=0}
        assertEquals(DocumentFailureKind.Verification,assertThrows(DocumentFailure::class.java){readDocumentRuntimePin(input)}.kind)
    }
}
