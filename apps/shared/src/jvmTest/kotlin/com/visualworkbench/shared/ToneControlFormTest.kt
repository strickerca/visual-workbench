package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class ToneControlFormTest {
    @Test fun defaultsRepresentANeutralEnabledToneAtFullOpacity() {
        val value = ToneControlForm().command(snapshot()) { id(7) to id(8) }
        assertEquals(ImageToneSettings(),value.settings)
        assertTrue(value.enabled); assertEquals(1.0,value.opacity,0.0)
    }

    @Test fun orderedLevelEndpointsAndEqualOutputLevelsAreValid() {
        val value = ToneControlForm(brightness="-1",contrast="1",inputBlack="0",inputWhite="1",
            gamma="0.001",outputBlack="0.5",outputWhite="0.5",opacity="0",enabled=false)
            .command(snapshot()) { id(7) to id(8) }
        assertEquals(-1.0,value.settings.brightness,0.0); assertEquals(1.0,value.settings.contrast,0.0)
        assertEquals(value.settings.outputBlack,value.settings.outputWhite,0.0)
        assertEquals(0.0,value.opacity,0.0); assertFalse(value.enabled)
    }

    @Test fun invalidValuesAreRefusedBeforeAnIdentityFactoryIsCalled() {
        var identities = 0
        val values = listOf(ToneControlForm(inputBlack="0.5",inputWhite="0.5"),ToneControlForm(inputBlack="-0.1"),
            ToneControlForm(outputWhite="1.01"),ToneControlForm(gamma="NaN"),ToneControlForm(opacity="Infinity"),
            ToneControlForm(gamma="1".repeat(33)))
        values.forEach { value ->
            try { value.command(snapshot()) { identities++; id(7) to id(8) }; fail("Expected invalid private form") }
            catch (error: WorkflowFailure) { assertEquals(WorkflowFailureKind.Invalid,error.kind) }
        }
        assertEquals(0,identities)
    }

    @Test fun savedToneRoundTripsParametersAndKeepsItsExactIds() {
        val saved = ToneControlEntry(id(7),id(8),false,false,0.3,ImageToneSettings(0.1,-0.2,0.2,0.9,1.3,0.1,0.8))
        val source = snapshot().copy(tones=listOf(saved))
        val value = source.form(saved.objectId).command(source) { error("Must reuse saved IDs") }
        assertEquals(saved.layerId,value.layerId); assertEquals(saved.objectId,value.objectId)
        assertEquals(saved.settings,value.settings); assertEquals(saved.opacity,value.opacity,0.0); assertFalse(value.enabled)
    }

    @Test fun missingOrLockedSavedToneCannotBecomeANewTone() {
        val source = snapshot().copy(tones=listOf(ToneControlEntry(id(7),id(8),true,true,1.0,ImageToneSettings())))
        listOf(id(8) to WorkflowFailureKind.Locked,id(9) to WorkflowFailureKind.Invalid).forEach { (objectId,kind) ->
            try { ToneControlForm(objectId).command(source) { error("Must not allocate a replacement") }; fail("Expected refused target") }
            catch (error: WorkflowFailure) { assertEquals(kind,error.kind) }
        }
    }

    @Test fun exactEdgePreviewRegionIsBoundedWithoutOverflowOrResizingTheSource() {
        val source = snapshot().copy(sourceWidth=UInt.MAX_VALUE,sourceHeight=UInt.MAX_VALUE)
        assertEquals(SelectionRegion(UInt.MAX_VALUE-1u,UInt.MAX_VALUE-1u,1u,1u),source.previewRegion(UInt.MAX_VALUE-1u,UInt.MAX_VALUE-1u))
        assertEquals(SelectionRegion(0u,0u,512u,512u),source.previewRegion(0u,0u))
        try { source.validateToneRegion(SelectionRegion(UInt.MAX_VALUE,0u,1u,1u)); fail("Expected overflowing edge refusal") }
        catch (error: WorkflowFailure) { assertEquals(WorkflowFailureKind.Invalid,error.kind) }
    }

    private fun snapshot(): ToneControlSnapshot = ToneControlSnapshot(WorkflowBinding(id(1),id(2),0uL,"a".repeat(64)),
        "b".repeat(64),1024u,768u,8u,emptyList())
    private fun id(number: Int): String = "00000000-0000-4000-8000-"+number.toString().padStart(12,'0')
}
