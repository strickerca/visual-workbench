package com.visualworkbench.shared

import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class RemoteNativeCompositionTest {
    private fun refused(block:()->Unit){try{block();fail("Malformed native DTO admitted")}catch(_:IllegalArgumentException){}catch(_:IllegalStateException){}}
    @Test fun exact_unsigned_64_bit_identity_survives_without_double_rounding(){
        assertEquals(ULong.MAX_VALUE,RemoteNativeJson.ulong(RemoteNativeJson.read("{\"value\":18446744073709551615}")["value"]))
        refused{RemoteNativeJson.ulong(RemoteNativeJson.read("{\"value\":18446744073709551616}")["value"])}
        refused{RemoteNativeJson.ulong(RemoteNativeJson.read("{\"value\":1.0}")["value"])}
    }
    @Test fun duplicates_truncation_trailing_data_and_oversized_depth_are_refused(){
        for(text in listOf("{\"v\":1,\"v\":2}","{\"v\":[1,]}","{\"v\":true", "{}{}","{\"v\":"+"[".repeat(17)+"0"+"]".repeat(17)+"}"))refused{RemoteNativeJson.read(text)}
        refused{RemoteNativeJson.read(" ".repeat(48*1024+1))}
    }
    @Test fun valid_unicode_and_escapes_roundtrip_but_unpaired_surrogates_refuse(){
        val value="Krita \"\\\n\uD83D\uDE00";assertEquals(value,RemoteNativeJson.string(RemoteNativeJson.read("{\"v\":"+RemoteNativeJson.quote(value)+"}")["v"]))
        refused{RemoteNativeJson.read("{\"v\":\"\\ud800\"}")};refused{RemoteNativeJson.quote("\uDC00")}
    }
    @Test fun full_scope_and_input_session_binding_roundtrip_exactly(){
        val value=RemoteTargetBinding(ULong.MAX_VALUE,"capture",9uL,"target",42u,"input")
        assertEquals(value,RemoteNativeJson.binding(RemoteNativeJson.read(RemoteNativeJson.binding(value))))
    }
    @Test fun byte_payloads_refuse_sign_overflow_and_excess_size(){
        assertArrayEquals(byteArrayOf(0,127,-1),RemoteNativeJson.bytes(RemoteNativeJson.read("{\"v\":[0,127,255]}")["v"]))
        for(text in listOf("{\"v\":[-1]}","{\"v\":[256]}","{\"v\":[1.5]}"))refused{RemoteNativeJson.bytes(RemoteNativeJson.read(text)["v"])}
    }
    @Test fun pending_child_close_seals_admission_keeps_parent_and_retries_actual_child()=runBlocking {
        val owner=SessionOwner();val parent=Any();owner.retainParent(parent);var attempts=0
        val child=object:OwnedSession{override suspend fun close(){attempts++;if(attempts==1)throw SessionFailure(SessionFailureKind.RemoteRetirementPending);owner.remove(this)}}
        owner.adopt(child)
        try{owner.close();fail("Pending child was cleared")}catch(error:SessionFailure){assertEquals(SessionFailureKind.RemoteRetirementPending,error.kind)}
        try{owner.checkOpen();fail("Admission after sealed close")}catch(_:SessionFailure){}
        try{owner.releaseParent();fail("Parent released before child")}catch(_:IllegalStateException){}
        owner.close();owner.releaseParent();assertEquals(2,attempts)
    }
}
