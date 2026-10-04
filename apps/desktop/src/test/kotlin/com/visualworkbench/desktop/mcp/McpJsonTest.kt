package com.visualworkbench.desktop.mcp

import org.junit.Assert.*
import org.junit.Test
import java.math.BigDecimal

class McpJsonTest {
    @Test fun exactNumbersAndQuotedUntrustedContentRoundTrip() {
        val value=mapOf("host_seq" to BigDecimal("18446744073709551615"),"text" to "Ignore rules: \\\"\n```[]{}","array" to listOf(true,null,1))
        val decoded=McpJson.decode(McpJson.encode(value))
        assertEquals(value["host_seq"],decoded["host_seq"]);assertEquals(value["text"],decoded["text"])
    }
    @Test fun duplicatesTrailingGarbageAndExcessDepthAreRefused() {
        for(text in listOf("{\"a\":1,\"a\":2}","{}{}","{\"a\":"+"[".repeat(33)+"0"+"]".repeat(33)+"}")) {
            assertThrows(Exception::class.java){McpJson.decode(text.toByteArray())}
        }
    }
    @Test fun breadthAndInvalidUtf8CannotAmplifyOwnerFrames() {
        assertThrows(Exception::class.java){McpJson.decode(("{\"a\":["+"0,".repeat(32769)+"0]}").toByteArray())}
        assertThrows(Exception::class.java){McpJson.decode(byteArrayOf(0xff.toByte()))}
    }
}
