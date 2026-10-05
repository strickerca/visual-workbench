package com.visualworkbench.shared

import kotlin.test.*

class RemoteEditorCompatibilityTest {
    private val digest="a".repeat(64)
    private fun identity()=RemoteEditorIdentity(RemoteEditorKind.Krita,"krita.exe",digest,1024uL,"5.3.4.0",null,null,digest)
    @Test fun package_and_file_versions_are_separate_facts(){
        val paint=identity().copy(editor=RemoteEditorKind.Paint,executableName="mspaint.exe",fileVersion="10.0.1.0",packageFullName="Microsoft.Paint_11.2605.81.0_x64__publisher",packageVersion="11.2605.81.0")
        assertTrue(paint.valid());assertFalse(paint.copy(packageVersion="10.0.1.0").valid());assertNull(remotePackageVersion("Paint_11.2.x.0_x64__publisher"))
    }
    @Test fun unknown_image_or_bad_digest_never_has_known_commands(){assertTrue(remoteEditorActions(RemoteEditorKind.Unknown).isEmpty());assertFalse(identity().copy(executableBlake3="filename").valid())}
    @Test fun compatibility_is_scoped_to_exact_image_tool_and_settings(){
        val record=RemoteToolCompatibility(identity(),"basic-brush",digest,RemoteEditorInputApi.WindowsPointerInput,RemoteCompatibilityAspect.Drawing,RemoteCompatibilityResult.Supported,RemoteCompatibilityMethod.EditorObserved,digest,null)
        assertTrue(record.validFor(identity(),"basic-brush",digest));assertFalse(record.validFor(identity().copy(fileVersion="5.3.5.0"),"basic-brush",digest));assertFalse(record.validFor(identity(),"other-tool",digest));assertFalse(record.validFor(identity(),"basic-brush","b".repeat(64)))
    }
    @Test fun sensor_success_requires_physical_device_observation(){
        val record=RemoteToolCompatibility(identity(),"brush",digest,RemoteEditorInputApi.WindowsPointerInput,RemoteCompatibilityAspect.Pressure,RemoteCompatibilityResult.Supported,RemoteCompatibilityMethod.EditorObserved,digest,null)
        assertFalse(record.validFor(identity(),"brush",digest));assertTrue(record.copy(method=RemoteCompatibilityMethod.PhysicalOwnerGesture,deviceModel="owner pen device").validFor(identity(),"brush",digest));assertFalse(record.copy(method=RemoteCompatibilityMethod.PhysicalOwnerGesture,deviceModel="owner pen device",inputApi=RemoteEditorInputApi.Mouse).validFor(identity(),"brush",digest))
    }
    @Test fun labels_do_not_claim_eraser_state_setter_or_local_undo(){
        assertTrue(remoteEditorActionLabel(RemoteEditorKind.Krita,RemoteEditorAction.ToggleEraserMode).startsWith("Toggle"));assertTrue(remoteEditorActionLabel(RemoteEditorKind.Krita,RemoteEditorAction.SelectFreehandBrush).contains("unchanged"));assertEquals("Undo in Paint",remoteEditorActionLabel(RemoteEditorKind.Paint,RemoteEditorAction.Undo))
    }
    @Test fun profile_evidence_requires_effect_and_guard_receipts(){
        val observation=RemoteEditorActionObservation(RemoteEditorAction.Undo,RemoteProfileRoute.PairedKeys,digest,1uL,100uL,true,digest)
        val record=RemoteEditorProfileEvidence(1u,identity(),"brush",digest,RemoteCanvasSelector("Qt","canvas","",50025,true),listOf(observation),digest,digest)
        assertTrue(record.validFor(identity(),"brush",digest));assertFalse(record.copy(observations=listOf(observation.copy(editorEffectObserved=false))).validFor(identity(),"brush",digest));assertFalse(record.copy(selector=record.selector.copy(requireKeyboardFocus=false)).validFor(identity(),"brush",digest));assertFalse(record.copy(guardLifecycleReceiptDigest="").validFor(identity(),"brush",digest))
    }
    @Test fun profile_digest_is_added_after_receipt_validation_without_a_self_hash_cycle(){
        val observation=RemoteEditorActionObservation(RemoteEditorAction.Undo,RemoteProfileRoute.PairedKeys,digest,1uL,100uL,true,digest)
        val record=RemoteEditorProfileEvidence(1u,identity().copy(shortcutProfileDigest=null),"brush",digest,RemoteCanvasSelector("Qt","canvas","",50025,true),listOf(observation),digest,digest)
        assertTrue(record.validFor(identity(),"brush",digest));assertFalse(record.validFor(identity().copy(executableBlake3="b".repeat(64)),"brush",digest))
    }
}
