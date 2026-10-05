package com.visualworkbench.android.remote

import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.*
import java.io.File
import java.nio.file.Files
import java.util.Base64
import java.util.UUID
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Opt-in integration variant only. No fake remote/codec/surface is injected.
 * Root supplies an ephemeral paired QR and one verified USB-only endpoint. */
@RunWith(AndroidJUnit4::class)
class RemoteNormalPathInstrumentedTest {
    @Test fun realUsbOwnedWgcHevcControllerRendersBackgroundAndReconnects(): Unit = runBlocking<Unit>(Dispatchers.Default) {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        val args=InstrumentationRegistry.getArguments()
        val id=requireNotNull(args.getString("remoteRunId"));check(id.matches(Regex("[a-f0-9]{32}")))
        var phoneOwnersStarting=false
        try {
        val app=instrumentation.targetContext
        check(app.packageName=="com.visualworkbench.android.hil"&&Build.MODEL=="SM-S918U")
        val controllerInput=args.getString("remoteControllerInput")=="true"
        check(args.getString("remoteControllerInput") in listOf(null,"true"))
        val inputScenario=args.getString("remoteInputScenario")?:"balanced"
        check(inputScenario in setOf("balanced","pause-held","background-held","disconnect-held"))
        check(controllerInput||inputScenario=="balanced")
        var heldReceipt:Bundle?=null
        val bootstrap=requireNotNull(args.getString("remoteBootstrap"))
        check(bootstrap.length in 1..8192)
        val fields=String(Base64.getDecoder().decode(bootstrap),Charsets.UTF_8).split('\n');check(fields.size==4)
        val qr=Base64.getDecoder().decode(fields[0]);val pairingEndpoint=fields[1];val projectId=fields[2];val endpoint=fields[3]
        val parent=app.noBackupFilesDir.canonicalFile;val root=File(parent,"remote-integration-$id")
        check(root.mkdir()&&root.canonicalFile.parentFile==parent&&!Files.isSymbolicLink(root.toPath()))
        val context=object:ContextWrapper(app){override fun getApplicationContext():Context=this;override fun getNoBackupFilesDir():File=root}
        var sessions:WorkbenchSessions?=null;var project:WorkbenchProject?=null;var link:ProjectLink?=null
        var remote:WorkbenchRemoteEdit?=null;var controller:RemoteEditController?=null;var scenario:ActivityScenario<RemoteIntegrationActivity>?=null
        var clean=false
        var retirementReport=0L
        fun pendingRetirement(){
            val now=System.nanoTime()
            if(now>=retirementReport){
                instrumentation.sendStatus(0,Bundle().apply{putString("remote_phase","waiting_actual_phone_input_retirement")})
                retirementReport=now+2_000_000_000L
            }
        }
        suspend fun retryClose(block:suspend()->Unit){
            val deadline=System.nanoTime()+10_000_000_000L
            while(true){try{block();return}catch(e:SessionFailure){if(e.kind!=SessionFailureKind.RemoteRetirementPending||(!controllerInput&&System.nanoTime()>=deadline))throw e
                    if(controllerInput)pendingRetirement()
                    delay(20)}}
        }
        fun phase(name:String){instrumentation.sendStatus(0,Bundle().apply{putString("remote_phase",name)})}
        var viewIndex=-1
        suspend fun attach(value:WorkbenchRemoteEdit):RemoteEditController {
            viewIndex++;check(viewIndex in 0..2)
            val instance=UUID.randomUUID().toString().replace("-","")
            RemoteIntegrationActivity.bind(instance,value)
            try {
                scenario=ActivityScenario.launch(Intent(app,RemoteIntegrationActivity::class.java).putExtra("ownedRunId",instance))
                var result:RemoteEditController?=null
                requireNotNull(scenario).onActivity{result=it.controller}
                return requireNotNull(result)
            }finally{RemoteIntegrationActivity.unbind(instance)}
        }
        // Opt-in, text-only diagnostics. No scope IDs, tickets, QR, endpoint,
        // destination label, arbitrary exception message or media bytes escape.
        fun diagnosticText(value:String?):String {
            if(value==null)return "none"
            if(value.length !in 1..256||!value.matches(Regex("[A-Za-z0-9 _.,:;()=-]+"))||
                Regex("(?:[0-9]{1,3}\\.){3}[0-9]{1,3}|(?:^|[ (])[A-Za-z]:|[A-Za-z0-9]{32,}").containsMatchIn(value))return "withheld"
            return value
        }
        fun renderDiagnostic(value:RemoteEditController,stage:String) {
            val display=value.display.value;val state=value.remote.state.value
            val decoder=display.decoder
            val nativeSnapshot=runCatching{remoteEditStreamDiagnostics(value.remote)}
            val carrierSnapshot=runCatching{val active=requireNotNull(link);active.status() to markerFocus(active).signal()}
            val decoderName=when(decoder){null->"absent";DecoderStatus.Starting->"Starting";is DecoderStatus.Ready->"Ready";is DecoderStatus.RecoveryRequired->"RecoveryRequired";DecoderStatus.Retiring->"Retiring";DecoderStatus.Retired->"Retired"}
            instrumentation.sendStatus(0,Bundle().apply{
                // Fixed numeric counters only. No scope, endpoint or media bytes.
                (nativeSnapshot.getOrDefault(emptyMap())+value.streamDiagnostics()).forEach { (key,count) ->
                    putString("remote_diag_stream_$key",count.toString())
                }
                putString("remote_diag_native_snapshot_available",nativeSnapshot.isSuccess.toString())
                putString("remote_diag_carrier_snapshot_available",carrierSnapshot.isSuccess.toString())
                carrierSnapshot.getOrNull()?.let{(status,focus)->
                    putString("remote_diag_carrier_status",status.status.name)
                    putString("remote_diag_carrier_failure",when(val reason=status.failure){null->"none";"untrusted_or_revoked","storage","capacity","timeout","invalid_peer_message","closed","connection","upgrade_required"->reason;else->"withheld"})
                    putString("remote_diag_carrier_epoch",focus.connectionEpoch.toString())
                    putString("remote_diag_carrier_available",focus.available.toString())
                }
                putString("remote_diag_view_index",viewIndex.toString())
                putString("remote_diag_stage",stage)
                putString("remote_diag_display_message",diagnosticText(display.message))
                putString("remote_diag_decoder",decoderName)
                putString("remote_diag_decoder_reason",diagnosticText((decoder as? DecoderStatus.RecoveryRequired)?.reason))
                putString("remote_diag_remote_status",state.status.name)
                putString("remote_diag_remote_reason",diagnosticText(state.reason))
                putString("remote_diag_config_present",(display.config!=null).toString())
                putString("remote_diag_scope_present",(state.scope!=null).toString())
                putString("remote_diag_viewport","${display.viewWidth}x${display.viewHeight}")
                display.renderIdentity?.let { putString("remote_diag_render_identity",it.boundedText()) }
            })
        }
        suspend fun rendered(value:RemoteEditController,prior:RemoteRenderedFrameObservation?,readyPhase:String):RemoteRenderedFrameObservation {
            val deadline=System.nanoTime()+30_000_000_000L;var next=System.nanoTime()
            val exactLink=requireNotNull(link);val focus=markerFocus(exactLink)
            var ready=false
            while(System.nanoTime()<deadline){
                val before=focus.signal();val status=exactLink.status();val after=focus.signal()
                if(before.available&&after.available&&before.connectionEpoch==after.connectionEpoch&&
                    before.connectionEpoch>0uL&&before.connectionEpoch and 1uL==1uL&&
                    status.status==SyncStatus.Synced&&status.carrier==SessionCarrier.QuicTether){ready=true;break}
                if(System.nanoTime()>=next){phase("waiting_fresh_carrier");renderDiagnostic(value,"waiting_fresh_carrier");next=System.nanoTime()+2_000_000_000L};delay(25)
            }
            if(!ready){renderDiagnostic(value,"render_timeout");error("Fresh native carrier not ready within render deadline")}
            phase(readyPhase);next=System.nanoTime()
            while(System.nanoTime()<deadline){
                val observed=value.renderedObservation.value
                if(observed!=null&&(prior==null||observed.scope!=prior.scope)) {
                    assertTrue(observed.ticket>0uL&&observed.frameId>0uL&&observed.ptsUs>0)
                    assertEquals(1uL,observed.generation);assertTrue(observed.callbackNanos>=observed.renderedNanos)
                    assertTrue(observed.admission.hardwareAccelerated&&observed.admission.sizeAnd30FpsAdvertised)
                    assertNull(observed.acknowledgment);assertFalse(requireNotNull(remote).target.value?.grantActive==true)
                    assertEquals(SessionCarrier.QuicTether,requireNotNull(link).status().carrier)
                    if(prior!=null){assertNotEquals(prior.ownerId,observed.ownerId);assertNotEquals(prior.scope.captureSessionId,observed.scope.captureSessionId)}
                    renderDiagnostic(value,"native_render_observed")
                    // These values came only after current decoder-owner render
                    // callback + actual native retained ticket/PTS validation.
                    instrumentation.sendStatus(0,Bundle().apply{
                        putString("remote_render_scope","${observed.scope.connectionEpoch},${observed.scope.captureSessionId},${observed.scope.sourceGeneration},${observed.scope.targetToken},${observed.scope.geometryRevision}")
                        putString("remote_render_owner",observed.ownerId);putString("remote_render_ticket",observed.ticket.toString())
                        putString("remote_render_frame",observed.frameId.toString());putString("remote_render_pts_us",observed.ptsUs.toString())
                        putString("remote_render_codec",observed.admission.codecName)
                        putString("remote_render_timing","${observed.renderedNanos},${observed.callbackNanos}")
                        putString("remote_render_capabilities","hardware=true,low_latency_advertised=${observed.admission.lowLatencyAdvertised},requested=${observed.admission.lowLatencyRequested},configure_accepted=${observed.admission.lowLatencyConfigureAccepted}")
                    })
                    return observed
                }
                val now=System.nanoTime();if(now>=next){phase("waiting_native_render_callback");renderDiagnostic(value,"waiting_native_render_callback");next=now+2_000_000_000L}
                delay(25)
            }
            renderDiagnostic(value,"render_timeout")
            error("Native/controller/surface callback phase timed out; no fallback codec/frame/ACK")
        }
        suspend fun actualGrant(first:RemoteRenderedFrameObservation):RemoteTargetBinding {
            val active=requireNotNull(controller);val native=requireNotNull(remote)
            native.requestControl()
            phase("controller_input_ready")
            val grantDeadline=System.nanoTime()+30_000_000_000L;var report=0L
            var binding:RemoteTargetBinding?=null
            while(System.nanoTime()<grantDeadline){
                val target=native.target.value;val mapped=active.renderedObservation.value
                if(target?.grantActive==true&&mapped!=null&&mapped.scope==first.scope&&mapped.frameId>first.frameId){binding=target.binding;break}
                val now=System.nanoTime();if(now>=report){renderDiagnostic(active,"waiting_actual_pc_grant_mapping");report=now+2_000_000_000L};delay(5)
            }
            return requireNotNull(binding){"Actual PC grant and current render mapping were not admitted"}
        }
        suspend fun controllerStylus(first:RemoteRenderedFrameObservation){
            val active=requireNotNull(controller);val native=requireNotNull(remote)
            val exact=actualGrant(first)
            var prior:RemoteRenderedFrameObservation?=null
            phase("controller_input_dispatch_attempted")
            requireNotNull(scenario).onActivity{prior=it.dispatchOwnedStylusBatch(exact)}
            phase("controller_input_dispatch_completed")
            val before=requireNotNull(prior)
            val admissionDeadline=System.nanoTime()+500_000_000L
            var observed:RemoteControllerInputObservation?=null
            while(System.nanoTime()<admissionDeadline){
                val value=active.inputEvidence().input
                if(value?.admission?.binding==exact&&value.phase==RemotePenPhase.Up){observed=value;break}
                check(native.target.value?.let{it.grantActive&&it.binding==exact}==true);delay(2)
            }
            val local=requireNotNull(observed){"Actual controller Up admission was not observed"}
            val segment=requireNotNull(local.ghostSegment)
            check(local.ghostAccepted&&local.admission.sequence==3uL&&segment.firstInputSequence==1uL&&segment.lastInputSequence==3uL)
            check(segment.binding==exact&&segment.points.count{!it.predicted&&!it.anchor}==3&&segment.points.none{it.anchor})
            var covered:RemoteControllerInputEvidence?=null
            val renderDeadline=System.nanoTime()+30_000_000_000L;var report=0L
            while(System.nanoTime()<renderDeadline){
                val evidence=active.inputEvidence();val frame=evidence.rendered;val ack=frame?.acknowledgment
                if(frame!=null&&ack?.binding==exact&&ack.lastInputSequenceApplied==local.admission.sequence){covered=evidence;break}
                check(native.target.value?.let{it.grantActive&&it.binding==exact}==true)
                val now=System.nanoTime();if(now>=report){renderDiagnostic(active,"waiting_native_covering_frame");report=now+2_000_000_000L};delay(2)
            }
            val evidence=requireNotNull(covered){"Native retained covering frame was not observed"}
            val frame=requireNotNull(evidence.rendered);val ack=requireNotNull(frame.acknowledgment)
            check(frame.ownerId==before.ownerId&&frame.scope==first.scope&&frame.frameId>before.frameId&&frame.ticket!=before.ticket&&frame.ptsUs>before.ptsUs)
            check(ack.frameId==frame.frameId&&ack.ticket==frame.ticket&&frame.callbackNanos>=frame.renderedNanos)
            val echo=evidence.echo;check(echo.binding==exact&&echo.observed==3uL&&echo.missing==0uL&&echo.pending==0&&echo.lastFrameId==frame.frameId&&echo.lastTicket==frame.ticket)
            // Observe actual phone-local fade; never advance the ghost clock
            // with fabricated time or confuse its 500ms hard expiry with ACK.
            val hardExpiry=segment.createdLocalNanos+500_000_000L
            val visible=evidence.ghosts.single{it.segment.firstInputSequence==segment.firstInputSequence}
            val fadeOrigin=requireNotNull(visible.acknowledgedLocalNanos)
            check(fadeOrigin<=frame.callbackNanos&&fadeOrigin+200_000_000L<hardExpiry)
            check(visible.segment.lastInputSequence==3uL&&visible.alpha in 0f..1f&&visible.alpha<1f)
            val fadeDeadline=fadeOrigin+200_000_000L
            var goneAt=0L
            while(System.nanoTime()<fadeDeadline){
                val now=System.nanoTime();check(now<hardExpiry)
                if(active.ghost.snapshot(now).none{it.segment.firstInputSequence==segment.firstInputSequence}){goneAt=now;break};delay(2)
            }
            check(goneAt>=fadeOrigin+150_000_000L&&goneAt<hardExpiry)
            instrumentation.sendStatus(0,Bundle().apply{
                putString("remote_input_binding","${exact.connectionEpoch},${exact.captureSessionId},${exact.sourceGeneration},${exact.targetToken},${exact.geometryRevision},${exact.inputSessionId}")
                putString("remote_input_sequences","${segment.firstInputSequence},${local.admission.sequence}")
                putString("remote_input_owner",frame.ownerId);putString("remote_input_frame",frame.frameId.toString());putString("remote_input_ticket",frame.ticket.toString());putString("remote_input_pts_us",frame.ptsUs.toString())
                putString("remote_input_timing","${segment.createdLocalNanos},${frame.renderedNanos},${frame.callbackNanos},$fadeOrigin,$goneAt")
                putString("remote_input_echo","${echo.observed},${echo.missing},${echo.pending}")
                putString("remote_input_route","surface_touch,software_generated=true,ghost_ack_fade=true,physical_pen_fidelity=false,editor_effect=false,latency_acceptance=false")
            })
            active.deactivate("owner_pause");native.pause("owner_pause")
            check(active.inputObservation.value==null&&active.ghost.snapshot(System.nanoTime()).isEmpty())
            phase("controller_input_complete")
        }
        suspend fun controllerHeld(first:RemoteRenderedFrameObservation){
            val active=requireNotNull(controller);val native=requireNotNull(remote)
            val exact=actualGrant(first);val rect=requireNotNull(native.state.value.clientRect)
            phase("controller_input_held_ready")
            val armed=File(root,"held-host-armed");val armedDeadline=System.nanoTime()+15_000_000_000L
            var armReport=0L
            while(!armed.isFile){
                check(System.nanoTime()<armedDeadline)
                val now=System.nanoTime();if(now>=armReport){phase("waiting_exact_host_witness_arm");armReport=now+2_000_000_000L};delay(5)
            }
            check(armed.canonicalFile.parentFile==root&&!Files.isSymbolicLink(armed.toPath())&&armed.length()==32L&&armed.readText()==id)
            if(inputScenario=="disconnect-held")armRemoteIntegrationCarrierFault(requireNotNull(link),id,exact)
            phase("controller_input_dispatch_attempted")
            requireNotNull(scenario).onActivity{it.dispatchOwnedStylusBatch(exact,held=true)}
            phase("controller_input_dispatch_completed")
            val admittedDeadline=System.nanoTime()+500_000_000L
            while(active.inputEvidence().input?.let{it.admission.binding==exact&&it.admission.sequence==2uL&&it.phase==RemotePenPhase.Move}!=true){
                check(System.nanoTime()<admittedDeadline);check(native.target.value?.let{it.grantActive&&it.binding==exact}==true);delay(2)
            }
            phase("controller_input_held_admitted")
            // Root releases this exact-run gate only after the real owned Windows
            // journal has Down+Move and no Up. No phone timestamp stands in for it.
            val gate=File(root,"held-native-ready");val gateDeadline=System.nanoTime()+15_000_000_000L;var report=0L
            while(!gate.isFile){
                check(System.nanoTime()<gateDeadline)
                val now=System.nanoTime();if(now>=report){phase("waiting_actual_windows_held_prefix");report=now+2_000_000_000L};delay(5)
            }
            check(gate.canonicalFile.parentFile==root.canonicalFile&&!Files.isSymbolicLink(gate.toPath())&&gate.length() in 1..512)
            val ready=gate.readLines();check(ready.size==8&&ready[0]=="M4_HELD_NATIVE_READY_V1"&&ready[1]==id&&ready[2]==inputScenario)
            check(ready[3].matches(Regex("[a-f0-9]{64}"))&&ready[4].toInt() in 1..262144&&ready[5]=="1"&&ready[6].toInt() in 1..20000&&ready[7]=="0")
            check(active.inputEvidence().input?.let{it.admission.sequence==2uL&&it.phase==RemotePenPhase.Move}==true)
            phase("controller_lifecycle_triggered")
            when(inputScenario){
                "pause-held"->{active.deactivate("owner_pause");native.pause("owner_pause")}
                "background-held"->requireNotNull(scenario).moveToState(Lifecycle.State.CREATED)
                // Explicit fixture feature closes only this exact armed carrier.
                // Normal connection loss owns release; no pause/Up/link close.
                "disconnect-held"->abortRemoteIntegrationCarrier(requireNotNull(link),id,exact)
                else->error("Held scenario required")
            }
            val revokeDeadline=System.nanoTime()+30_000_000_000L;report=0L
            while(native.target.value?.let{it.grantActive&&it.binding==exact}==true){
                check(System.nanoTime()<revokeDeadline)
                val now=System.nanoTime();if(now>=report){phase("waiting_actual_held_grant_revocation");report=now+2_000_000_000L};delay(5)
            }
            check(active.inputObservation.value==null&&active.ghost.snapshot(System.nanoTime()).isEmpty())
            // Negative production admission checks are distinct from the omitted
            // phone Up. Only Authentication/Closed establishes retired binding.
            for(tail in listOf(RemotePenPhase.Move,RemotePenPhase.Up)){
                val sample=RemotePenSample(tail,rect.x+rect.width.toInt()/2,rect.y+rect.height.toInt()/2,if(tail==RemotePenPhase.Up)0u else 512u,0,0,0u,0u,System.nanoTime())
                var refused=false
                try{native.pen(exact,sample)}catch(error:SessionFailure){check(error.kind in setOf(SessionFailureKind.Authentication,SessionFailureKind.Closed));refused=true}
                check(refused){"Stale held-contact tail was admitted"}
            }
            heldReceipt=Bundle().apply{
                putString("remote_lifecycle_scenario",inputScenario)
                putString("remote_lifecycle_binding","${exact.connectionEpoch},${exact.captureSessionId},${exact.sourceGeneration},${exact.targetToken},${exact.geometryRevision},${exact.inputSessionId}")
                putString("remote_lifecycle_sequences","1,2")
                putString("remote_lifecycle_phone_up_admitted","false")
                putString("remote_lifecycle_native_prefix_sha256",ready[3])
                putString("remote_lifecycle_native_prefix_bytes",ready[4])
                putString("remote_lifecycle_stale_tail_refused","true")
                putString("remote_lifecycle_route","surface_touch,software_generated=true,host_safety_release=true,physical_pen_fidelity=false")
            }
            phase("controller_input_complete")
        }
        suspend fun retireView(){
            controller?.let{runCatching{renderDiagnostic(it,"before_phone_owner_retirement")}}
            controller?.invalidate()
            val deadline=System.nanoTime()+10_000_000_000L
            while(controller?.retire()==false){if(!controllerInput)check(System.nanoTime()<deadline);else pendingRetirement();delay(20)}
            if(controller!=null)assertTrue(controller?.actualRetired==true)
            controller?.let{runCatching{renderDiagnostic(it,"after_decoder_retirement")}}
            scenario?.close();scenario=null
            retryClose{remote?.close()};remote=null;controller=null
            while(RemoteIntegrationActivity.pendingOwners()!=0){if(!controllerInput)check(System.nanoTime()<deadline);else pendingRetirement();delay(20)}
        }
        phoneOwnersStarting=true
        try {
            // This marker is inside the existing retained-owner finally. Even
            // reporting failure cannot leave an attempted session unowned.
            phase("phone_owners_starting")
            withTimeout(150_000) {
                sessions=createAndroidSessions(context,workbenchCore().newDeviceId())
                val peer=requireNotNull(sessions).joinQr(qr,pairingEndpoint);qr.fill(0)
                val routes=listOf(SessionEndpoint(SessionCarrier.QuicTether,endpoint))
                val receiveDeadline=System.nanoTime()+15_000_000_000L
                while(project==null){
                    try{project=requireNotNull(sessions).receiveProject(File(root,"project").absolutePath,peer,routes,projectId,64uL*1024uL*1024uL)}
                    catch(e:SessionFailure){
                        if(e.kind !in setOf(SessionFailureKind.Transport,SessionFailureKind.Timeout)||System.nanoTime()>=receiveDeadline)throw e
                        phase("waiting_exact_host_listener");delay(100)
                    }
                }
                link=requireNotNull(sessions).connect(requireNotNull(project),peer,routes)
                remote=remoteEdit(requireNotNull(link));controller=attach(requireNotNull(remote))
                val first=rendered(requireNotNull(controller),null,"initial_controller_ready");phase("initial_rendered")
                if(controllerInput){if(inputScenario=="balanced")controllerStylus(first)else controllerHeld(first)}
                scenario?.moveToState(Lifecycle.State.CREATED)
                assertNull(controller?.renderedObservation?.value)
                retireView()
                heldReceipt?.putString("remote_lifecycle_phone_settled","true")
                remote=remoteEdit(requireNotNull(link));controller=attach(requireNotNull(remote))
                val second=rendered(requireNotNull(controller),first,"background_retired");phase("background_resumed_rendered")
                if(inputScenario!="balanced")check(requireNotNull(remote).target.value?.grantActive!=true)
                retireView();retryClose{link?.close()};link=null
                link=requireNotNull(sessions).connect(requireNotNull(project),peer,routes)
                remote=remoteEdit(requireNotNull(link));controller=attach(requireNotNull(remote))
                val third=rendered(requireNotNull(controller),second,"link_reconnected")
                assertNotEquals(first.scope.connectionEpoch,third.scope.connectionEpoch)
                phase("reconnected_rendered")
                heldReceipt?.let{receipt->
                    check(requireNotNull(remote).target.value?.grantActive!=true)
                    receipt.putString("remote_lifecycle_return_granted","false")
                    instrumentation.sendStatus(0,receipt)
                }
            }
        } finally {
            withContext(NonCancellable+Dispatchers.Default){
                qr.fill(0)
                // Each successful release clears its owner only after return. A
                // query/error cannot discard the remaining concrete owners.
                while(true){
                    try{
                        retireView();retryClose{link?.close()};link=null
                        project?.close();project=null;sessions?.close();sessions=null
                        assertRemoteEditRetired();check(RemoteIntegrationActivity.pendingOwners()==0)
                        break
                    }catch(t:Throwable){if(!controllerInput)throw t;pendingRetirement();delay(20)}
                }
                check(root.canonicalFile.parentFile==parent&&root.name=="remote-integration-$id")
                root.walkTopDown().forEach{check(!Files.isSymbolicLink(it.toPath()))}
                check(root.deleteRecursively());clean=true
                phase("actual_phone_owners_retired")
            }
            assertTrue(clean)
        }
        } finally {
            if(!phoneOwnersStarting){
                // Exact-run failed-startup evidence only: no session, decoder,
                // Surface or native remote owner creation has been attempted.
                // The runner must still prove process/package cleanup; this is
                // never a rendered-frame or normal-retirement success marker.
                instrumentation.sendStatus(0,Bundle().apply{
                    putString("remote_no_owner_run_id",id)
                    putString("remote_phase","actual_phone_no_owners_created")
                })
            }
        }
    }
}
