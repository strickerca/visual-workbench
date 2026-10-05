package com.visualworkbench.desktop

import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.assertRemoteEditRetired
import com.visualworkbench.bindings.host.setProcessPerMonitorV2
import java.nio.file.*
import java.util.Base64
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.*

/** Explicit central-test entry point, separate from normal Main. It uses the
 * real DLL session/WGC/MFT routes and an isolated memory trust store. No profile,
 * owner project or Windows trust registry is changed. The explicit opt-in input
 * marker enables one ordinary local grant into the exact owned guard target. */
internal object RemoteIntegrationHost {
    private val retained=ConcurrentHashMap.newKeySet<Any>()
    private class Startup {
        var protectedOwnersEntered=false
        var runtime:DesktopNativeRuntime?=null
    }
    @JvmStatic fun main(args:Array<String>):Unit = runBlocking<Unit>(Dispatchers.Default) {
        // A separate invocation nonce permits bound no-owner proof even when
        // config/runtime initialization fails before the owner finally exists.
        require(args.size==2&&args[1].matches(Regex("[a-f0-9]{32}")))
        val runId=args[1];val startup=Startup()
        try { runOwned(args[0],runId,startup) }
        catch(t:Throwable) {
            if(!startup.protectedOwnersEntered) {
                // This branch precedes every Cancellation/session/link owner.
                // Extraction leases must actually close before emitting proof.
                startup.runtime?.close();startup.runtime=null
                println("REMOTE_HOST_NO_OWNERS:run=$runId;pid=${ProcessHandle.current().pid()}")
            }
            throw t
        }
    }
    private suspend fun runOwned(configArgument:String,runId:String,startup:Startup) {
        val config=Path.of(configArgument).toAbsolutePath().normalize()
        require(Files.isRegularFile(config,LinkOption.NOFOLLOW_LINKS)&&Files.size(config) in 1..8192)
        val fields=Files.readAllLines(config,Charsets.UTF_8)
        require(fields.size==12&&fields[0]=="M4_REMOTE_INTEGRATION_V1")
        val root=config.parent
        require(root.fileName.toString().matches(Regex("VisualWorkbench-remote-integration-[a-f0-9]{32}")))
        require(root.fileName.toString().removePrefix("VisualWorkbench-remote-integration-")==runId)
        require(Files.readString(root.resolve(".owner-v1"))==runId)
        WindowsNativeFileGuard().check(root)
        val address=fields[1]
        require(address.matches(Regex("(?:[0-9]{1,3}\\.){3}[0-9]{1,3}")))
        val bytes=address.split('.').map{it.toInt()};require(bytes.all{it in 0..255}&&bytes[0]!=0&&bytes[0]!=127&&bytes[0]<224)
        val window=fields[2].toULong();val pid=fields[3].toUInt();val created=fields[4].toULong()
        require(window>0uL&&pid>0u&&created>0uL)
        val port=fields[5].toInt();require(port in 1024..65535)
        val names=listOf("vw_core.dll","vw_host.dll","vw-connection-helper.exe","vw-capture-helper.exe","vw-hevc-helper.exe","vw-input-helper.exe")
        val hashes=fields.drop(6);require(hashes.all{it.matches(Regex("[a-f0-9]{64}"))})
        val runtime=DesktopNativeRuntime.prepare(Path.of(System.getenv("LOCALAPPDATA")?:error("Native cache unavailable"))).also{startup.runtime=it}
        require(names.zip(hashes).all{runtime.packagedHash(it.first)==it.second})
        runtime.activate()
        check(setProcessPerMonitorV2().perMonitorV2)
        val trust=MemoryTrust();var service:SessionService?=null;var pairing:PairingServer?=null
        var project:ProjectSession?=null;var link:LiveSession?=null;var clean=false
        val inputMarker=root.resolve("controller-input-owner")
        val controllerInput=Files.isRegularFile(inputMarker,LinkOption.NOFOLLOW_LINKS)
        if(controllerInput){WindowsNativeFileGuard().check(inputMarker);require(Files.size(inputMarker)==32L&&Files.readString(inputMarker)==runId)}
        val heldScenarioPath=root.resolve("held-scenario")
        val heldScenario=if(Files.exists(heldScenarioPath,LinkOption.NOFOLLOW_LINKS)){
            require(controllerInput);WindowsNativeFileGuard().check(heldScenarioPath)
            require(Files.size(heldScenarioPath) in 1..64)
            Files.readString(heldScenarioPath).also{require(it in setOf("pause-held","background-held","disconnect-held"))}
        }else null
        var witnessWritten=false
        var inputAttempted=false;var inputGranted:Boolean?=false;var inputComplete=false
        var cancellation:Cancellation?=null;var nativeOwnersStarted=false
        var failure:Throwable?=null
        val diagnostics=RetirementDiagnostics(waitingPhase=controllerInput)
        startup.protectedOwnersEntered=true
        try {
            println("REMOTE_HOST_PHASE:host_owners_starting")
            cancellation=Cancellation()
            withTimeout(180_000) {
                // From this point only the existing actual-retirement finally
                // owns cleanup; a startup exception cannot claim no owners.
                nativeOwnersStarted=true
                runtime.armRetirementFence(::assertRemoteEditRetired)
                service=openCallbackSessionService(generateDeviceId(),trust)
                val local=requireNotNull(service).localDevice()
                val projectId=generateId(System.currentTimeMillis().toULong())
                val source=Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4////fwAJ+wP9KobjigAAAABJRU5ErkJggg==")
                project=createImageProject(CreateImageProject(root.resolve("project").toString(),projectId,
                    generateId(System.currentTimeMillis().toULong()),generateId(System.currentTimeMillis().toULong()),
                    local.deviceId,"Owned remote video fixture",source,System.currentTimeMillis()),requireNotNull(cancellation))
                source.fill(0)
                pairing=requireNotNull(service).listenPairing("$address:0")
                val offer=requireNotNull(pairing).offer(false)
                require(offer.qr.isNotEmpty()&&offer.code.isEmpty())
                // Secret exists only in the fresh private run folder; runner
                // transfers it without console/log output and disposes it.
                publishFile(root,"offer.txt",listOf(Base64.getEncoder().encodeToString(offer.qr),offer.endpoint,projectId).joinToString("\n"))
                offer.qr.fill(0)
                println("REMOTE_HOST_PHASE:pairing_listening")
                val paired=requireNotNull(pairing).accept(requireNotNull(cancellation))
                require(paired.confirmation==null);val peer=requireNotNull(paired.pairedDeviceId)
                Files.deleteIfExists(root.resolve("offer.txt"))
                pairing?.closeSession();pairing?.destroy();pairing=null
                link=requireNotNull(service).hostProject(requireNotNull(project),peer,listOf(SessionEndpoint(AppCarrier.QUIC_TETHER,"$address:$port")))
                val active=requireNotNull(link)
                active.remoteConfigureRuntime(RemoteRuntimeFiles(runtime.directory.resolve("vw-hevc-helper.exe").toString(),runtime.packagedHash("vw-hevc-helper.exe"),
                    runtime.directory.resolve("vw-input-helper.exe").toString(),runtime.packagedHash("vw-input-helper.exe"),null,null))
                require(active.endpoints().single()==SessionEndpoint(AppCarrier.QUIC_TETHER,"$address:$port"))
                publishFile(root,"endpoint.txt",active.endpoints().single().address)
                var next=0;var lastReport=System.nanoTime();var selectedCarrierEpoch=0uL
                while(!Files.exists(root.resolve("stop"),LinkOption.NOFOLLOW_LINKS)) {
                    val status=active.status()
                    if(status.carrier!=null)require(status.carrier==AppCarrier.QUIC_TETHER)
                    if(controllerInput&&next==1&&inputGranted!=true&&Files.isRegularFile(root.resolve("input-ready"),LinkOption.NOFOLLOW_LINKS)){
                        require(Files.readString(root.resolve("input-ready"))==runId&&status.status==SyncStatus.SYNCED&&status.carrier==AppCarrier.QUIC_TETHER)
                        val selected=active.remoteWindows().singleOrNull{it.window==window&&it.processId==pid&&it.processCreated==created}?:error("Owned input target changed")
                        require(selected.executableName=="vw-remote-guard-hil.exe")
                        // Exactly the normal local PC grant operation. OS focus,
                        // target snapshot, helper admission and every pen batch
                        // retain their production guards; any refusal fails.
                        val jobGate=root.resolve("host-input-job-ready")
                        WindowsNativeFileGuard().check(jobGate)
                        require(Files.isRegularFile(jobGate,LinkOption.NOFOLLOW_LINKS)&&Files.size(jobGate) in 1..1024)
                        val gate=Files.readAllLines(jobGate)
                        val self=ProcessHandle.current();val parent=self.parent().orElseThrow()
                        require(gate==listOf("M4_INPUT_JOB_V1",runId,runId,self.pid().toString(),
                            self.info().startInstant().orElseThrow().toEpochMilli().toString(),parent.pid().toString(),
                            parent.info().startInstant().orElseThrow().toEpochMilli().toString(),"assigned"))
                        inputAttempted=true;inputGranted=null
                        println("REMOTE_HOST_PHASE:owned_input_grant_attempted")
                        val grantDiagnostics=GrantDiagnostics(heldScenario!=null)
                        grantDiagnostics.at("before_grant")
                        try{
                            active.remoteGrant()
                            grantDiagnostics.at("native_grant_returned")
                            val grantedDisplay=active.remoteDisplay()
                            require(grantedDisplay.status=="controlling")
                            inputGranted=true
                            println("REMOTE_HOST_PHASE:actual_owned_input_granted")
                            grantDiagnostics.controlling(grantedDisplay.status,grantedDisplay.reason)
                            if(heldScenario!=null){
                                grantDiagnostics.at("witness_arm_started")
                                active.remoteIntegrationArmReleaseWitness(runId,requireNotNull(active.remoteDisplay().bindingJson),heldScenario)
                                grantDiagnostics.armed()
                                publishFile(root,"held-witness-armed",runId)
                            }
                            grantDiagnostics.at("ready")
                        }catch(t:Throwable){
                            grantDiagnostics.failed(t){active.remoteDisplay().let{it.status to it.reason}}
                            throw t
                        }
                    }
                    if(controllerInput&&inputGranted==true&&!inputComplete&&Files.isRegularFile(root.resolve("input-complete"),LinkOption.NOFOLLOW_LINKS)){
                        require(Files.readString(root.resolve("input-complete"))==runId)
                        inputComplete=true
                        println("REMOTE_HOST_PHASE:owned_controller_input_completed")
                    }
                    if(heldScenario!=null&&inputGranted==true&&!witnessWritten){
                        active.remoteIntegrationReleaseWitness(runId)?.let{witness->
                            require(witness.length in 1..4096)
                            publishFile(root,"held-release-witness.json",witness);witnessWritten=true
                        }
                    }
                    val request=if(next==0)"initial" else "restart-$next"
                    val carrier=active.focusSignal()
                    if(Files.isRegularFile(root.resolve(request),LinkOption.NOFOLLOW_LINKS)&&
                        remoteIntegrationCarrierReady(next,selectedCarrierEpoch,carrier.connectionEpoch,carrier.available,
                            status.status==SyncStatus.SYNCED,status.carrier==AppCarrier.QUIC_TETHER)) {
                        // A reconnect marker is consumed only by a genuinely newer
                        // native carrier. Old SYNCED state cannot spend restart-2.
                        val checked=active.focusSignal()
                        if(!checked.available||checked.connectionEpoch!=carrier.connectionEpoch)continue
                        val target=active.remoteWindows().singleOrNull{it.window==window&&it.processId==pid&&it.processCreated==created}
                            ?:error("Owned selected window changed")
                        // No grant or input is synthesized. Actual production
                        // selection admits the exact retained physical window.
                        active.remoteSelectWindow(target.window,target.processId,target.processCreated)
                        val selectedScope=active.remoteDisplay().scopeJson?:error("Capture scope unavailable")
                        val afterSelection=active.focusSignal()
                        require(afterSelection.available&&afterSelection.connectionEpoch==carrier.connectionEpoch)
                        selectedCarrierEpoch=carrier.connectionEpoch
                        println("REMOTE_HOST_SELECTION:index=$next;epoch=$selectedCarrierEpoch")
                        publishFile(root,"selected-$next",selectedScope)
                        next++;require(next<=3)
                        println("REMOTE_HOST_PHASE:capture_selected")
                    }
                    if(System.nanoTime()-lastReport>=2_000_000_000L){
                        val display=active.remoteDisplay()
                        println("REMOTE_HOST_PHASE:waiting_for_owned_phone_phase")
                        println("REMOTE_HOST_DIAGNOSTIC:status=${diagnosticCode(display.status)};reason=${diagnosticCode(display.reason)};configured=${display.encoderDescription!=null};scope=${display.scopeJson!=null};link=${status.status.name}")
                        println("REMOTE_HOST_STREAM:"+active.remoteStreamDiagnostics())
                        reportCarrier(active)
                        lastReport=System.nanoTime()
                    }
                    delay(25)
                }
                require(next==3&&(!controllerInput||(inputGranted==true&&inputComplete))&&(heldScenario==null||witnessWritten))
            }
        } catch(t:Throwable){failure=t;diagnostics.runFailed(t);throw t}
        finally {
            withContext(NonCancellable+Dispatchers.Default) {
                // Terminal aggregate snapshot precedes retirement; it never
                // establishes render success or actual ownership settlement.
                runCatching{link?.let{println("REMOTE_HOST_STREAM:"+it.remoteStreamDiagnostics())}}
                link?.let{reportCarrier(it)}
                if(!controllerInput){
                cancellation?.cancel()
                val deadline=System.nanoTime()+10_000_000_000L
                try {
                    while(true) {
                        try {diagnostics.attempt();link?.remoteClose();link?.closeSession();break}
                        catch(e:SessionException){diagnostics.failed(e);if(e !is SessionException.RemoteRetirementPending||System.nanoTime()>=deadline)throw e;delay(20)}
                    }
                    link?.destroy();link=null
                    pairing?.closeSession();pairing?.destroy();pairing=null
                    project?.closeSession();project?.destroy();project=null
                    if(nativeOwnersStarted)require(remoteHelpersRetirementCount()==0u)
                    service?.destroy();service=null;trust.clear();cancellation?.destroy();cancellation=null;runtime.close();clean=true
                    diagnostics.completed()
                } catch(t:Throwable) {
                    diagnostics.failed(t)
                    // A failed query/join is not retirement. Root's bounded Job
                    // runner owns this JVM/tree; keep all owners until actual exit.
                    retained.add(listOfNotNull(link,pairing,project,service,cancellation,runtime,trust))
                    if(failure!=null)failure?.addSuppressed(t) else throw t
                } finally {
                    Files.deleteIfExists(root.resolve("offer.txt"))
                    publishFile(root,"host-receipt.txt","schema=1\nactual_owners_retired=$clean\ninput_granted=false\neditor_effect=false\nlatency_acceptance=false\n")
                }
                }else{
                cancellation?.cancel()
                var remoteClosed=false;var sessionClosed=false
                while(true){
                    diagnostics.attempt()
                    try{
                        if(!remoteClosed){link?.remoteClose();remoteClosed=true}
                        if(!sessionClosed){link?.closeSession();sessionClosed=true}
                        if(nativeOwnersStarted)require(remoteHelpersRetirementCount()==0u)
                        link?.destroy();link=null
                        pairing?.closeSession();pairing?.destroy();pairing=null
                        project?.closeSession();project?.destroy();project=null
                        service?.destroy();service=null;trust.clear();cancellation?.destroy();cancellation=null;runtime.close();clean=true
                        diagnostics.completed()
                        break
                    }catch(t:Throwable){
                        diagnostics.failed(t)
                        delay(20)
                    }
                }
                Files.deleteIfExists(root.resolve("offer.txt"))
                publishFile(root,"host-receipt.txt","schema=1\nrun_id=$runId\nhost_pid=${ProcessHandle.current().pid()}\nactual_owners_retired=$clean\ninput_grant_attempted=$inputAttempted\ninput_granted=$inputGranted\ncontroller_input_complete=$inputComplete\neditor_effect=false\nlatency_acceptance=false\n")
                }
            }
        }
    }
    private fun reportCarrier(active:LiveSession){
        val snapshot=runCatching{active.status() to active.focusSignal()}
        runCatching{println(snapshot.fold({(status,focus)->
            "REMOTE_HOST_CARRIER:status=${status.status.name};failure=${carrierFailureCode(status.failure)};epoch=${focus.connectionEpoch};available=${focus.available}"
        },{"REMOTE_HOST_CARRIER_UNAVAILABLE"}))}
    }
    internal fun carrierFailureCode(value:String?):String=when(value){
        null->"none"
        "untrusted_or_revoked","storage","capacity","timeout","invalid_peer_message","closed","connection","upgrade_required"->value
        else->"withheld"
    }
    // Diagnostics carry no ownership authority. Output failure cannot replace
    // the original exception or interrupt release-only retirement retries.
    internal class RetirementDiagnostics(private val output:(String)->Unit={println(it)},private val clock:()->Long=System::nanoTime,private val waitingPhase:Boolean=false) {
        private var first="none";private var current="none"
        private var attempts=0L;private var failures=0L;private var lastReport:Long?=null
        private var runFailureReported=false;private var attemptFailed=false
        fun runFailed(error:Throwable){if(!runFailureReported){runFailureReported=true;emit("REMOTE_HOST_FAILURE:code=${failureCode(error)}")}}
        fun attempt(){attempts=increment(attempts);attemptFailed=false}
        fun failed(error:Throwable){
            if(attemptFailed)return
            attemptFailed=true
            current=failureCode(error);if(first=="none")first=current
            failures=increment(failures)
            val now=clock();val previous=lastReport
            if(previous==null||now-previous>=2_000_000_000L){lastReport=now;report()}
        }
        fun completed(){current="none";report()}
        private fun report(){if(waitingPhase&&current!="none")emit("REMOTE_HOST_PHASE:waiting_actual_input_retirement");emit("REMOTE_HOST_RETIREMENT:first=$first;current=$current;attempts=$attempts;failures=$failures")}
        private fun emit(line:String){runCatching{output(line)}}
        private fun increment(value:Long)=if(value==Long.MAX_VALUE)value else value+1
    }
    // Fixed-field observation only. A query/output failure cannot replace the
    // originating grant/arm error or acquire input/retirement authority.
    internal class GrantDiagnostics(private val held:Boolean,private val output:(String)->Unit={println(it)}) {
        private var stage="before_grant";private var confirmed=false;private var armed=false
        fun at(value:String,status:String?=null,reason:String?=null){stage=value;report("none",status,reason,status!=null)}
        fun controlling(status:String,reason:String?){confirmed=true;at("controlling_observed",status,reason)}
        fun armed(){armed=true;at("witness_armed")}
        fun failed(error:Throwable,snapshot:()->Pair<String,String?>){
            val actual=runCatching{snapshot()}.getOrNull()
            report(failureCode(error),actual?.first,actual?.second,actual!=null)
        }
        private fun report(code:String,status:String?,reason:String?,available:Boolean){
            val witness=if(!held)"not_requested" else if(armed)"armed" else if(stage=="witness_arm_started")"arming" else "unarmed"
            runCatching{output("REMOTE_HOST_GRANT:stage=$stage;code=$code;grant=${if(confirmed)"confirmed" else "unknown"};witness=$witness;display_available=$available;status=${grantStatusCode(status)};reason=${if(available)grantReasonCode(reason) else "unavailable"}")}
        }
    }
    internal fun grantStatusCode(value:String?):String=when(value){
        null->"unavailable"
        "selecting","viewing","controlling","pending_focus","pending_grant","paused","sealed","disconnected","closing","closed"->value
        else->"withheld"
    }
    internal fun grantReasonCode(value:String?):String=when(value){
        null->"none"
        "target_focus_required","connection_retired","host_revoked","peer_background","owner_pause","background","input_expired","input_worker_unavailable","video_worker_unavailable","partial_input"->value
        else->"withheld"
    }
    internal fun failureCode(error:Throwable):String=when(error){
        is SessionException.Invalid->"invalid"
        is SessionException.Authentication->"authentication"
        is SessionException.Closed->"closed"
        is SessionException.RemoteRetirementPending->"pending"
        is SessionException.RemotePartialInput->"partial_input"
        is SessionException.Backpressure->"backpressure"
        is SessionException.Timeout,is TimeoutCancellationException->"timeout"
        is SessionException.Transport->"transport"
        is SessionException.Worker->"worker"
        is SessionException.RemoteUnavailable->"unavailable"
        is SessionException->"session_other"
        is CancellationException->"cancelled"
        is IllegalArgumentException->"argument"
        is IllegalStateException->"state"
        is AssertionError->"assertion"
        is java.io.IOException->"io"
        else->"other"
    }
    private fun diagnosticCode(value:String?):String = value?.takeIf{
        it.length in 1..96&&it.matches(Regex("[A-Za-z][A-Za-z0-9_.:-]*"))&&
        !Regex("(?:[0-9]{1,3}\\.){3}[0-9]{1,3}|(?:^|[ (])[A-Za-z]:|[A-Za-z0-9]{32,}").containsMatchIn(it)
    }?:if(value==null)"none" else "withheld"
    internal fun publishFile(root:Path,name:String,value:String){
        require(name.matches(Regex("[a-z0-9.-]{1,40}")))
        val pending=root.resolve("$name.pending");val destination=root.resolve(name)
        java.nio.channels.FileChannel.open(pending,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use { channel ->
            val bytes=java.nio.ByteBuffer.wrap(value.toByteArray(Charsets.UTF_8))
            while(bytes.hasRemaining())channel.write(bytes)
            channel.force(true)
        }
        // A hard-link publication exposes only closed complete bytes and fails
        // if the final name exists. No replacement/partial readiness is allowed.
        Files.createLink(destination,pending)
        // Publication already succeeded; failure to remove the private staged
        // link cannot revoke readiness or replace the published final bytes.
        runCatching{Files.delete(pending)}
    }
    private class MemoryTrust:ProtectedTrustCallback {
        private var bytes:ByteArray?=null;private var revision=0uL;private var loaded:ByteArray?=null
        @Synchronized override fun load():ByteArray?{loaded?.fill(0);return bytes?.copyOf().also{loaded=it}}
        @Synchronized override fun clearLoadedPlaintext(){loaded?.fill(0);loaded=null}
        @Synchronized override fun compareExchange(expectedRevision:ULong,replacementRevision:ULong,plaintext:ByteArray):Boolean {
            try {require(plaintext.size in 1..2*1024*1024&&replacementRevision==expectedRevision+1uL)
                if(revision!=expectedRevision)return false
                bytes?.fill(0);bytes=plaintext.copyOf();revision=replacementRevision;return true
            }finally{plaintext.fill(0)}
        }
        @Synchronized fun clear(){bytes?.fill(0);bytes=null;clearLoadedPlaintext()}
    }
}
