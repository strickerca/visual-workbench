package com.visualworkbench.desktop

import com.visualworkbench.shared.GiantInputInfo
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantProducerBoundaryTest {
    @Test fun uncertainProducerNeverDeletesSourceOrPrivateWork()=runBlocking {
        val calls=mutableListOf<String>()
        try{settleGiantResources({calls+="producer";throw IllegalStateException("uncertain")},{calls+="source"},{calls+="files"});fail("expected")}
        catch(_:IllegalStateException){}
        assertEquals(listOf("producer"),calls)
    }
    @Test fun sourceRetirementPrecedesPrivateCleanupAndIsNotAssumed()=runBlocking {
        val calls=mutableListOf<String>()
        try{settleGiantResources({calls+="producer"},{calls+="source";throw IllegalStateException("uncertain")},{calls+="files"});fail("expected")}
        catch(_:IllegalStateException){}
        assertEquals(listOf("producer","source"),calls)
        calls.clear();settleGiantResources({calls+="producer"},{calls+="source"},{calls+="files"})
        assertEquals(listOf("producer","source","files"),calls)
    }
    @Test fun diskAdmissionIncludesBothImmutableCopies(){
        val valid=GiantInputInfo("private","a".repeat(64),"b".repeat(64),102_470_031uL,16320u,12240u,16320u,12240u,1u,"jpeg")
        assertEquals(valid.encodedBytes*2uL+199_756_800uL*12uL+64uL*1024uL*1024uL,admitGiantWorkingDisk(valid))
        try{admitGiantWorkingDisk(valid.copy(encodedBytes=4uL*1024uL*1024uL*1024uL,width=100000u,height=10000u));fail("expected")}
        catch(_:HandoffFailure){}
    }
}
