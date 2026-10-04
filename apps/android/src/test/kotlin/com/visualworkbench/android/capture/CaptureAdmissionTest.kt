package com.visualworkbench.android.capture

import org.junit.Assert.*
import org.junit.Test

class CaptureAdmissionTest {
    @Test fun largeOrOverflowingDimensionsRefusedBeforeAllocation(){
        for(pair in listOf(0 to 1,Int.MAX_VALUE to Int.MAX_VALUE,10000 to 10000,6000 to 6000)){
            try{CaptureAdmission.pixels(pair.first,pair.second);fail("admitted")}catch(_:CaptureRefusal){}
        }
        assertEquals(1920L*1080*4,CaptureAdmission.pixels(1920,1080))
    }
    @Test fun timestampsRecordSignedObservedDifference(){assertEquals(15,CaptureAdmission.delta(1000,1015));assertEquals(-15,CaptureAdmission.delta(1015,1000))}
    @Test fun unicodeExcerptUsesScalarBoundWithoutCopyingDocument(){val value="\uD83D\uDE00".repeat(1000);val actual=CaptureAdmission.excerpt(value);assertEquals(200,actual.codePointCount(0,actual.length))}
    @Test fun outputLimitIsEnforcedDuringEncoding(){val output=LimitedOutput(java.io.ByteArrayOutputStream(),3);output.write(byteArrayOf(1,2,3));try{output.write(4);fail("overflow")}catch(_:CaptureRefusal){}}
}
