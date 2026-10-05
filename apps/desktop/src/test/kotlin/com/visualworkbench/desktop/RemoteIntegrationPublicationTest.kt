package com.visualworkbench.desktop

import java.io.ByteArrayOutputStream
import java.io.PrintStream
import java.nio.file.Files
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class RemoteIntegrationPublicationTest {
    @get:Rule val temporary=TemporaryFolder()
    @Test fun publishedPayloadIsCompleteAndStagedLinkIsGone(){
        val root=temporary.newFolder().toPath();val value="owned\n".repeat(1024)
        RemoteIntegrationHost.publishFile(root,"offer.txt",value)
        assertEquals(value,Files.readString(root.resolve("offer.txt")))
        assertFalse(Files.exists(root.resolve("offer.txt.pending")))
    }
    @Test fun existingFinalIsNeverReplaced(){
        val root=temporary.newFolder().toPath();Files.writeString(root.resolve("offer.txt"),"prior")
        assertThrows(java.nio.file.FileAlreadyExistsException::class.java){RemoteIntegrationHost.publishFile(root,"offer.txt","replacement")}
        assertEquals("prior",Files.readString(root.resolve("offer.txt")))
        assertEquals("replacement",Files.readString(root.resolve("offer.txt.pending")))
    }
    @Test fun incompletePriorStagingCannotBecomeReady(){
        val root=temporary.newFolder().toPath();Files.writeString(root.resolve("offer.txt.pending"),"partial")
        assertThrows(java.nio.file.FileAlreadyExistsException::class.java){RemoteIntegrationHost.publishFile(root,"offer.txt","complete")}
        assertFalse(Files.exists(root.resolve("offer.txt")))
        assertEquals("partial",Files.readString(root.resolve("offer.txt.pending")))
    }
    @Test fun invalidConfigProducesBoundNoOwnerProofBeforeNativeLoading(){
        val nonce="0123456789abcdef0123456789abcdef"
        val output=captureOutput {
            assertThrows(IllegalArgumentException::class.java){RemoteIntegrationHost.main(arrayOf(temporary.root.toPath().resolve("missing-config").toString(),nonce))}
        }
        assertEquals("REMOTE_HOST_NO_OWNERS:run=$nonce;pid=${ProcessHandle.current().pid()}",output.trim())
    }
    @Test fun invalidInvocationNonceCannotProduceOwnerProof(){
        val output=captureOutput {
            assertThrows(IllegalArgumentException::class.java){RemoteIntegrationHost.main(arrayOf("missing-config","not-a-run"))}
        }
        assertEquals("",output)
    }
    @Test fun retirementDiagnosticsKeepFirstCauseAndBoundedProgress(){
        val lines=mutableListOf<String>();var now=0L
        val state=RemoteIntegrationHost.RetirementDiagnostics({lines.add(it)},{now})
        state.runFailed(IllegalArgumentException("secret"));state.runFailed(AssertionError("later"))
        state.attempt();state.failed(IllegalStateException("private"))
        state.attempt();state.failed(java.io.IOException("secret"))
        assertEquals(2,lines.size)
        now=2_000_000_000L;state.attempt();state.failed(java.io.IOException("secret"))
        assertEquals("REMOTE_HOST_RETIREMENT:first=state;current=io;attempts=3;failures=3",lines.last())
        state.attempt();state.completed()
        assertEquals("REMOTE_HOST_RETIREMENT:first=state;current=none;attempts=4;failures=3",lines.last())
        assertEquals("REMOTE_HOST_FAILURE:code=argument",lines.first())
        assertFalse(lines.any{it.contains("secret")||it.contains("private")||it.contains("later")})
    }
    @Test fun diagnosticOutputFailureCannotInterruptRetirement(){
        val state=RemoteIntegrationHost.RetirementDiagnostics({throw IllegalStateException("output")},{0L})
        state.runFailed(AssertionError());state.attempt();state.failed(java.io.IOException());state.attempt();state.completed()
    }
    @Test fun diagnosticUnknownTypeNeverPublishesClassOrMessage(){
        assertEquals("other",RemoteIntegrationHost.failureCode(object:Throwable("private"){}))
        assertEquals("closed",RemoteIntegrationHost.failureCode(com.visualworkbench.bindings.core.SessionException.Closed()))
        assertEquals("pending",RemoteIntegrationHost.failureCode(com.visualworkbench.bindings.core.SessionException.RemoteRetirementPending()))
    }
    @Test fun carrierDiagnosticCodesExcludeArbitraryFailureText(){
        assertEquals("none",RemoteIntegrationHost.carrierFailureCode(null))
        assertEquals("untrusted_or_revoked",RemoteIntegrationHost.carrierFailureCode("untrusted_or_revoked"))
        assertEquals("withheld",RemoteIntegrationHost.carrierFailureCode("private endpoint body"))
    }
    @Test fun grantErrorsKeepTypedInvalidAndAuthentication(){
        assertEquals("invalid",RemoteIntegrationHost.failureCode(com.visualworkbench.bindings.core.SessionException.Invalid()))
        assertEquals("authentication",RemoteIntegrationHost.failureCode(com.visualworkbench.bindings.core.SessionException.Authentication()))
    }
    @Test fun rejectedNativeGrantRemainsUnknownAndPreservesDisplayReason(){
        val lines=mutableListOf<String>();val state=RemoteIntegrationHost.GrantDiagnostics(true){lines.add(it)}
        state.at("before_grant")
        state.failed(com.visualworkbench.bindings.core.SessionException.Authentication()){"pending_focus" to "target_focus_required"}
        assertEquals("REMOTE_HOST_GRANT:stage=before_grant;code=authentication;grant=unknown;witness=unarmed;display_available=true;status=pending_focus;reason=target_focus_required",lines.last())
    }
    @Test fun witnessFailureCannotErasePreviouslyConfirmedGrant(){
        val lines=mutableListOf<String>();val state=RemoteIntegrationHost.GrantDiagnostics(true){lines.add(it)}
        state.at("before_grant");state.at("native_grant_returned");state.controlling("controlling",null);state.at("witness_arm_started")
        state.failed(com.visualworkbench.bindings.core.SessionException.Invalid()){"controlling" to null}
        assertEquals("REMOTE_HOST_GRANT:stage=witness_arm_started;code=invalid;grant=confirmed;witness=arming;display_available=true;status=controlling;reason=none",lines.last())
    }
    @Test fun grantSnapshotAndOutputFailuresCannotReplaceOriginatingError(){
        val lines=mutableListOf<String>();val state=RemoteIntegrationHost.GrantDiagnostics(true){lines.add(it)}
        state.failed(com.visualworkbench.bindings.core.SessionException.Authentication()){throw IllegalStateException("private")}
        assertEquals("REMOTE_HOST_GRANT:stage=before_grant;code=authentication;grant=unknown;witness=unarmed;display_available=false;status=unavailable;reason=unavailable",lines.single())
        RemoteIntegrationHost.GrantDiagnostics(true){throw IllegalStateException("output")}.failed(IllegalArgumentException()){throw IllegalStateException("snapshot")}
    }
    @Test fun grantDiagnosticsWithholdUnknownTextAndKeepWitnessDistinct(){
        val lines=mutableListOf<String>();val state=RemoteIntegrationHost.GrantDiagnostics(true){lines.add(it)}
        state.controlling("controlling",null);state.at("witness_arm_started");state.armed();state.at("ready")
        assertTrue(lines.last().contains("grant=confirmed;witness=armed"))
        state.failed(AssertionError("private")){"private/path" to "https://private.invalid"}
        assertTrue(lines.last().endsWith("display_available=true;status=withheld;reason=withheld"))
        assertFalse(lines.any{it.contains("private")})
        val ordinary=RemoteIntegrationHost.GrantDiagnostics(false){lines.add(it)}
        ordinary.controlling("controlling",null);ordinary.at("ready")
        assertTrue(lines.last().contains("grant=confirmed;witness=not_requested"))
    }
    private fun captureOutput(block:()->Unit):String=synchronized(RemoteIntegrationHost){
        val previous=System.out;val bytes=ByteArrayOutputStream()
        PrintStream(bytes,true,Charsets.UTF_8).use { output ->
            try{System.setOut(output);block()}finally{System.setOut(previous)}
        }
        bytes.toString(Charsets.UTF_8)
    }
}
