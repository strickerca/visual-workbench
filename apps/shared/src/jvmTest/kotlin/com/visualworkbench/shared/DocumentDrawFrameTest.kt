package com.visualworkbench.shared

import kotlinx.coroutines.*
import java.util.concurrent.CountDownLatch
import kotlin.test.*

class DocumentDrawFrameTest {
    private val revision=ProjectInfo("project","title","device",1uL,false,false,1uL,"b".repeat(64),listOf("doc"))
    private val binding=DocumentFrameBinding("a".repeat(64),"doc",0u,revision,1,1)
    private class Image:AutoCloseable { var closes=0;var fail=false;override fun close(){if(fail)throw IllegalStateException("fixture");closes++} }
    @Test fun replacementDoesNotReleaseAnActualRendererReader():Unit=runBlocking {
        val image=Image();val frame=DocumentDrawFrame(binding,listOf(image));val reader=assertNotNull(frame.acquire())
        frame.close();assertFalse(frame.retired);assertNull(frame.acquire());assertEquals(0,image.closes)
        assertEquals(1,reader.read{it.size});reader.close();frame.awaitRetired();assertEquals(1,image.closes)
        reader.close();frame.close();assertEquals(1,image.closes)
    }
    @Test fun unpublishedReplacementCanRetireWithoutARenderer():Unit=runBlocking {
        val image=Image();val frame=DocumentDrawFrame(binding,listOf(image));frame.close();frame.awaitRetired();assertEquals(1,image.closes);assertNull(frame.acquire())
    }
    @Test fun aDrawAlreadyInProgressRetainsItsOwnReadAcrossCompositionDisposal():Unit=runBlocking {
        val image=Image();val frame=DocumentDrawFrame(binding,listOf(image));val reader=assertNotNull(frame.acquire())
        val entered=CountDownLatch(1);val release=CountDownLatch(1)
        val draw=async(Dispatchers.Default){reader.read{entered.countDown();check(release.await(5,java.util.concurrent.TimeUnit.SECONDS));it.size}}
        try { withContext(Dispatchers.IO){check(entered.await(5,java.util.concurrent.TimeUnit.SECONDS))};frame.close();reader.close();assertEquals(0,image.closes);assertFalse(frame.retired) }
        finally{release.countDown()}
        assertEquals(1,draw.await());frame.awaitRetired();assertEquals(1,image.closes)
    }
    @Test fun failedDisposalRetainsTheSameImageAndCanRetry():Unit=runBlocking {
        val image=Image().apply{fail=true};val frame=DocumentDrawFrame(binding,listOf(image))
        assertEquals(DocumentFailureKind.Isolation,assertFailsWith<DocumentFailure>{frame.close()}.kind)
        assertFalse(frame.retired);assertFalse(frame.available);image.fail=false;frame.close();frame.awaitRetired();assertEquals(1,image.closes)
    }
    @Test fun finalCleanupRegistryRetainsItsOwnerUntilActualSuccess():Unit=runBlocking {
        val owner=Any();var attempts=0;var fail=true
        DocumentRetirementRegistry.retain(owner){attempts++;if(fail)throw DocumentFailure(DocumentFailureKind.Isolation)}
        try{assertEquals(DocumentFailureKind.Isolation,assertFailsWith<DocumentFailure>{DocumentRetirementRegistry.drain()}.kind);assertEquals(1,attempts)}
        finally{fail=false;DocumentRetirementRegistry.drain()}
        assertEquals(2,attempts);DocumentRetirementRegistry.drain();assertEquals(2,attempts)
    }
}
