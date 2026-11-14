package com.visualworkbench.android.remote

import com.visualworkbench.android.diagnostics.InputDeviceCapability
import com.visualworkbench.android.diagnostics.InputDeviceRange
import com.visualworkbench.shared.RemotePenPhase
import org.junit.Assert.*
import org.junit.Test

class RemotePhysicalGestureJournalTest {
    // A real built-in pen MOVE: stylus+touchscreen digitizer, window-delivered, unsigned action.
    private val physical=RemoteInputProvenance(7,0x5002,false,true,false,0x5002,REMOTE_TOOL_TYPE_STYLUS,0,0,0,0,1,123_000_000L,
        2,true,RemoteInputVerification.NotSigned)
    private val down=physical.copy(actionMasked=REMOTE_ACTION_DOWN,verification=RemoteInputVerification.Verified)
    private fun row(seq:Int,phase:RemotePenPhase,origin:RemoteInputOrigin=RemoteInputOrigin.PhysicalStylus,event:Int=0)=
        RemotePhysicalJournalRow(seq.toULong(),event,phase,100+seq,200,512u,0.5f,0.25f,1.5f,0f,3,-4,7u,0u,1_000L+seq,false,origin)
    private val header=RemotePhysicalJournalHeader("0123456789abcdef0123456789abcdef","pressure","SM-S918U",36,1L,0L,12L,3uL,3uL,0uL,0)

    @Test fun onlyWindowDeliveredSignedPenDigitizerInputWithoutSyntheticDispatchIsPhysical() {
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(physical,0L))
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(down,0L))
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(down.copy(actionMasked=REMOTE_ACTION_UP),0L))
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(physical.copy(toolType=REMOTE_TOOL_TYPE_ERASER),0L))
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(physical.copy(source=0xD002,deviceSources=0xD002),0L))
    }
    @Test fun directViewDispatchIsNeverPhysicalEvenWithPhysicalLookingFields() {
        assertEquals(RemoteInputOrigin.Direct,classifyRemoteInput(physical.copy(windowDelivered=false),0L))
        assertEquals(RemoteInputOrigin.Direct,classifyRemoteInput(down.copy(windowDelivered=false),0L))
        // The existing synthetic producer: MotionEvent.obtain(deviceId 0) dispatched straight into the view.
        val synthetic=RemoteInputProvenance(0,null,null,false,null,0x4002,REMOTE_TOOL_TYPE_STYLUS,0,0,0,0,0,1L,0,false,RemoteInputVerification.NotDelivered)
        assertEquals(RemoteInputOrigin.Direct,classifyRemoteInput(synthetic,1L))
    }
    @Test fun injectedOrUnsignedSourcesAreRefusedBeforeAnyStylusPredicate() {
        assertEquals(RemoteInputOrigin.Injected,classifyRemoteInput(physical.copy(deviceId=-1),0L))
        assertEquals(RemoteInputOrigin.Injected,classifyRemoteInput(physical.copy(deviceVirtual=true),0L))
        assertEquals(RemoteInputOrigin.Injected,classifyRemoteInput(physical.copy(flags=REMOTE_MOTION_FLAG_ACCESSIBILITY),0L))
        assertEquals(RemoteInputOrigin.Injected,classifyRemoteInput(physical.copy(flags=REMOTE_MOTION_FLAG_ACCESSIBILITY or 0x2),0L))
        assertEquals(RemoteInputOrigin.Injected,classifyRemoteInput(physical.copy(flags=REMOTE_MOTION_FLAG_GENERATED_GESTURE),0L))
        // An in-process Activity.dispatchTouchEvent call fails the dispatcher signature.
        assertEquals(RemoteInputOrigin.Injected,classifyRemoteInput(down.copy(verification=RemoteInputVerification.Refused),0L))
    }
    @Test fun shellStylusInjectionShapeIsNeverPhysical() {
        // `input stylus swipe`: real stylus-capable deviceId, flags 0, SOURCE_STYLUS only,
        // dispatcher-signed. The pen-digitizer source combination is absent.
        val shell=down.copy(source=REMOTE_SOURCE_STYLUS)
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(shell,0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(shell.copy(actionMasked=2,verification=RemoteInputVerification.NotSigned),0L))
    }
    @Test fun unprovenDeviceToolSourceSignatureOrContaminatedRunStaysUnknown() {
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical,1L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(deviceVirtual=null),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(declaresStylus=false),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(toolType=1),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(toolType=3),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(source=0x1002),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(deviceSources=null),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(physical.copy(deviceSources=0x4002),0L))
        // API 29 has no verifier: a signed action is never promoted.
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(down.copy(verification=RemoteInputVerification.Unavailable),0L))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(down.copy(verification=RemoteInputVerification.NotSigned),0L))
    }
    @Test fun eventOverflowStopsAcceptingAndMarksIncomplete() {
        val journal=RemotePhysicalGestureJournal(1,false,maxEvents=2,maxSamples=8)
        assertEquals(0 to RemoteInputOrigin.PhysicalStylus,journal.recordEvent(physical,0,0L))
        assertEquals(1 to RemoteInputOrigin.Direct,journal.recordEvent(physical.copy(windowDelivered=false),2,0L))
        assertNull(journal.recordEvent(physical,2,0L))
        val progress=journal.progress()
        assertFalse(progress.accepting);assertFalse(progress.complete);assertEquals("event_limit",progress.stopReason)
        assertNull(journal.recordEvent(physical,2,0L))
        assertEquals(2,journal.snapshot().events.size)
    }
    @Test fun sampleOverflowIsExplicitlyIncompleteNeverSilentlyTruncated() {
        val journal=RemotePhysicalGestureJournal(4,false,maxEvents=8,maxSamples=3)
        listOf(RemotePenPhase.Down,RemotePenPhase.Move,RemotePenPhase.Move,RemotePenPhase.Up).forEachIndexed{i,p->journal.admitted(row(i+1,p))}
        val snapshot=journal.snapshot()
        assertEquals(3,snapshot.samples.size);assertFalse(snapshot.complete);assertEquals("sample_limit",snapshot.stopReason)
        assertFalse(journal.accepting())
        assertTrue(snapshot.toJson(header).contains("\"complete\":false,\"stop_reason\":\"sample_limit\""))
    }
    @Test fun contactStrokeTargetClosesAfterTheQueuedUpOnly() {
        val journal=RemotePhysicalGestureJournal(2,false)
        for(p in listOf(RemotePenPhase.Hover,RemotePenPhase.Leave,RemotePenPhase.Down,RemotePenPhase.Move,RemotePenPhase.Up,RemotePenPhase.Hover,RemotePenPhase.Down,RemotePenPhase.Move))journal.queued(p)
        assertTrue(journal.accepting())
        journal.queued(RemotePenPhase.Up)
        assertFalse(journal.accepting());assertEquals("target_reached",journal.progress().stopReason);assertTrue(journal.progress().complete)
        // An Up without its Down is not a stroke.
        val orphan=RemotePhysicalGestureJournal(1,false);orphan.queued(RemotePenPhase.Up);assertTrue(orphan.accepting())
    }
    @Test fun hoverPassesCountOnlyWithoutContact() {
        val journal=RemotePhysicalGestureJournal(2,true)
        for(p in listOf(RemotePenPhase.Hover,RemotePenPhase.Down,RemotePenPhase.Up,RemotePenPhase.Leave))journal.queued(p)
        assertTrue(journal.accepting())
        for(p in listOf(RemotePenPhase.Hover,RemotePenPhase.Hover,RemotePenPhase.Leave,RemotePenPhase.Hover))journal.queued(p)
        assertTrue(journal.accepting())
        journal.queued(RemotePenPhase.Leave)
        assertFalse(journal.accepting())
        for((i,p)in listOf(RemotePenPhase.Hover,RemotePenPhase.Leave,RemotePenPhase.Hover,RemotePenPhase.Hover,RemotePenPhase.Leave).withIndex())journal.admitted(row(i+1,p))
        assertEquals(2,journal.progress().admittedStrokes)
    }
    @Test fun admittedStrokesFollowNativeRowsNotQueuedBatches() {
        val journal=RemotePhysicalGestureJournal(3,false)
        for((i,p)in listOf(RemotePenPhase.Down,RemotePenPhase.Move,RemotePenPhase.Up,RemotePenPhase.Hover,RemotePenPhase.Down,RemotePenPhase.Up).withIndex())journal.admitted(row(i+1,p))
        val progress=journal.progress()
        assertEquals(2,progress.admittedStrokes);assertEquals(6,progress.samples);assertEquals(1uL,progress.firstSequence);assertEquals(6uL,progress.lastSequence)
    }
    @Test fun ownerPauseReasonsAreFixedLabels() {
        val journal=RemotePhysicalGestureJournal(1,false)
        journal.pause("tool_type");journal.pause("tool_type");journal.pause("C:\\private\\path")
        assertEquals(mapOf("tool_type" to 2L,"unclassified" to 1L),journal.snapshot().pauses)
        assertEquals(3,journal.progress().pauses)
        val text=journal.snapshot().toJson(header)
        assertFalse(text.contains("private"))
        assertTrue(text.contains("{\"reason\":\"tool_type\",\"count\":2}"))
    }
    @Test fun censusAndJsonCarryNoDeviceNameOrDescriptor() {
        val journal=RemotePhysicalGestureJournal(1,false,maxRanges=2)
        val ranges=(0 until 5).map{InputDeviceRange(it,0x4002,0f,1f,0f,0f,0f)}
        journal.recordDevice(InputDeviceCapability(7,0x5002,true,false,false,ranges))
        journal.recordDevice(InputDeviceCapability(7,0,false,true,true,emptyList()))
        journal.recordEvent(physical,0,0L);journal.admitted(row(1,RemotePenPhase.Down))
        val snapshot=journal.snapshot()
        assertEquals(1,snapshot.devices.size);assertEquals(2,snapshot.devices.single().ranges.size)
        val fieldNames=InputDeviceCapability::class.java.declaredFields.map{it.name.lowercase()}+
            RemoteInputProvenance::class.java.declaredFields.map{it.name.lowercase()}
        for(forbidden in listOf("name","descriptor","vendor","product","serial"))assertTrue(fieldNames.none{forbidden in it})
        val text=snapshot.toJson(header)
        for(forbidden in listOf("\"name\"","descriptor","vendor","product","serial"))assertFalse(text.contains(forbidden))
        assertTrue(text.startsWith("{\"schema\":1,\"kind\":\"remote-physical-gesture-journal\",\"run_id\":\"0123456789abcdef0123456789abcdef\",\"block\":\"pressure\""))
        assertTrue(text.contains("\"input_devices\":[{\"device_id\":7,\"sources\":20482,\"declares_stylus\":true,\"virtual\":false,\"external\":false,\"ranges\":[{\"axis\":0,"))
        assertTrue(text.contains("\"events\":[{\"event\":0,\"action\":0,\"device_id\":7,\"device_virtual\":false,\"declares_stylus\":true,\"external\":false,\"device_sources\":20482,\"source\":20482,\"tool_type\":2,\"flags\":0,\"button_state\":0,\"action_button\":0,\"classification\":0,\"history_size\":1,\"event_time_ns\":123000000,\"window_delivered\":true,\"verification\":\"NotSigned\",\"origin\":\"PhysicalStylus\"}]"))
        assertTrue(text.contains("\"samples\":[{\"seq\":1,\"event\":0,\"phase\":\"Down\",\"host_x\":101,\"host_y\":200,\"sent_pressure\":512,\"raw_pressure\":0.5,\"raw_tilt\":0.25,\"raw_orientation\":1.5,\"raw_distance\":0.0,\"tilt_x\":3,\"tilt_y\":-4,\"pen_mask\":7,\"pen_flags\":0,\"sampled_ns\":1001,\"historical\":false,\"origin\":\"PhysicalStylus\"}]"))
        assertTrue(text.endsWith(",\"owner_pauses\":[]}"))
    }
    @Test fun nonFiniteAxesSerializeAsNullAndHeaderRefusesUnboundedText() {
        val journal=RemotePhysicalGestureJournal(1,false)
        journal.admitted(row(1,RemotePenPhase.Hover).copy(rawTilt=Float.NaN,rawOrientation=null,rawDistance=Float.POSITIVE_INFINITY))
        val text=journal.snapshot().toJson(header)
        assertTrue(text.contains("\"raw_tilt\":null,\"raw_orientation\":null,\"raw_distance\":null"))
        assertFalse(text.contains("NaN")||text.contains("Infinity"))
        assertTrue(runCatching{journal.snapshot().toJson(header.copy(runId="../x"))}.isFailure)
        assertTrue(runCatching{journal.snapshot().toJson(header.copy(block="tilt"))}.isFailure)
        assertTrue(runCatching{journal.snapshot().toJson(header.copy(deviceModel="SM S918U\""))}.isFailure)
        assertTrue(runCatching{RemotePhysicalGestureJournal(0,false)}.isFailure)
    }
}
