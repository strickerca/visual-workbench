package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class ImageFramesFormTest {
    @Test fun originalDefaultAndExplicitIndependentOutputSizesPreserveSourceCoordinates() {
        val source = snapshot()
        val defaults = source.form()
        assertEquals("1024",defaults.cropWidth); assertEquals("768",defaults.cropHeight)
        val form = defaults.copy(cropX="20",cropY="30",cropWidth="400",cropHeight="300",clockwiseDegrees=90u,
            outputWidth="71",outputHeight="19")
        val command = form.command(source) { id(3) to id(4) }
        assertEquals(ImageFrameSettings(20u,30u,400u,300u,90u,71u,19u),command.settings)
        assertEquals("71",form.outputWidth); assertEquals("19",form.outputHeight)
        assertEquals(1024u,source.sourceWidth); assertEquals(768u,source.sourceHeight)
    }

    @Test fun invalidWholeFormsNeverConsumeIdentitiesIncludingOverflowAndDisabledFrames() {
        val source = snapshot(); val defaults = source.form(); var ids = 0
        val invalid = listOf(defaults.copy(cropX="-1"),defaults.copy(cropY="1.0"),defaults.copy(cropWidth="0"),
            defaults.copy(cropHeight=""),defaults.copy(outputWidth="0"),defaults.copy(outputHeight="32769"),
            defaults.copy(outputWidth="10000",outputHeight="5001"),defaults.copy(cropX=UInt.MAX_VALUE.toString()),
            defaults.copy(cropWidth="1025"),defaults.copy(cropY="768"),defaults.copy(clockwiseDegrees=45u),
            defaults.copy(outputWidth="1e2"),defaults.copy(outputWidth=" 1"),defaults.copy(outputWidth="4294967296"),
            defaults.copy(outputHeight="1".repeat(33)),defaults.copy(enabled=false,cropHeight="0"))
        invalid.forEach { form ->
            try { form.command(source) { ids++; id(3) to id(4) }; fail("Invalid complete frame admitted: $form") }
            catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Invalid,failure.kind) }
        }
        assertEquals(0,ids)
    }

    @Test fun exactLimitsAndSavedIdentityAreRetainedAndLocksRefuseTheWholeEdit() {
        val settings = ImageFrameSettings(0u,0u,1024u,768u,270u,32768u,1u)
        val saved = ImageFrameEntry(id(3),id(4),false,false,settings)
        val source = snapshot(saved)
        var ids = 0
        val command = source.form().copy(outputWidth="10000",outputHeight="5000").command(source) {
            ids++; id(5) to id(6)
        }
        assertEquals(id(3),command.layerId); assertEquals(id(4),command.objectId)
        assertFalse(command.enabled); assertEquals(0,ids)
        val locked = source.copy(frame=saved.copy(locked=true))
        try { locked.form().command(locked) { ids++; id(5) to id(6) }; fail("Locked frame admitted") }
        catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Locked,failure.kind) }
        assertEquals(0,ids)
    }

    @Test fun disabledFrameUsesOriginalOutputGridAndEditingPreviewAlwaysUsesSourceGrid() {
        val settings = ImageFrameSettings(100u,100u,200u,100u,90u,80u,50u)
        val source = snapshot(ImageFrameEntry(id(3),id(4),false,false,settings))
        assertEquals(1024u,source.outputWidth); assertEquals(768u,source.outputHeight)
        assertNull(source.effectiveFrame(null))
        val region = SelectionRegion(900u,700u,20u,20u)
        val pixels = ByteArray(20*20*4)
        val editing = ImageEditingPreview(source.binding,source.sourceAssetId,region.x,region.y,region.width,region.height,pixels)
        val owned = editing.ownedImageEditingPreview(source.copy(frame=source.frame!!.copy(enabled=true)),region)
        pixels.fill(9)
        assertEquals(0.toByte(),owned.rgba[0])
        assertTrue(owned.toString().contains("rgbaBytes=1600")); assertFalse(owned.toString().contains("rgba=["))
    }

    @Test fun outputPreviewMustMatchEffectivePrivateFrameRatherThanSavedOrSourceDimensions() {
        val source = snapshot(); val settings = ImageFrameSettings(0u,0u,500u,400u,180u,700u,10u)
        val command = ImageFrameCommand.Set(id(3),id(4),settings,true)
        val region = SelectionRegion(600u,0u,100u,10u)
        val preview = ImageFramePreview(source.binding,source.sourceAssetId,settings,700u,10u,600u,0u,100u,10u,ByteArray(4000))
        val owned = preview.ownedImageFramePreview(source,command,region)
        preview.rgba.fill(7); assertEquals(0.toByte(),owned.rgba[0])
        val corruptions = listOf(preview.copy(frame=null),preview.copy(outputWidth=1024u),preview.copy(sourceAssetId="d".repeat(64)),
            preview.copy(binding=source.binding.copy(hostSeq=3uL)),preview.copy(x=0u),preview.copy(rgba=ByteArray(4)))
        corruptions.forEach { value ->
            try { value.ownedImageFramePreview(source,command,region); fail("Mismatched frame preview admitted") }
            catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Stale,failure.kind) }
        }
        try { validateImageFrameRegion(SelectionRegion(UInt.MAX_VALUE,0u,2u,1u),1024u,768u); fail("Overflow region admitted") }
        catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Invalid,failure.kind) }
    }

    @Test fun malformedSnapshotCannotPublishUnboundedOrOutOfSourceSavedFrame() {
        val valid = snapshot()
        val malformed = listOf(valid.copy(sourceWidth=0u),valid.copy(sourceBitDepth=32u),valid.copy(sourceAssetId="z".repeat(64)),
            valid.copy(frame=ImageFrameEntry(id(3),id(4),true,false,ImageFrameSettings(UInt.MAX_VALUE,0u,2u,1u,0u,1u,1u))))
        malformed.forEach { value ->
            try { value.ownedImageFramesSnapshot(); fail("Malformed snapshot admitted") }
            catch (_: WorkflowFailure) { }
        }
    }

    @Test fun wholeEditingBackgroundIsOwnedBoundedAndNeverLogsSamplesOrProfile() {
        val source = snapshot()
        val background = ImageEditingBackground(source.binding,source.sourceAssetId,16u,2u,3u,ByteArray(24),byteArrayOf(1,2))
        val owned = background.ownedImageEditingBackground(source.binding,26uL)
        background.rgba.fill(9); background.iccProfile.fill(9)
        assertEquals(0.toByte(),owned.rgba[0]); assertEquals(1.toByte(),owned.iccProfile[0])
        assertTrue(owned.toString().contains("rgbaBytes=24")); assertTrue(owned.toString().contains("iccProfileBytes=2"))
        assertFalse(owned.toString().contains("rgba=[")); assertFalse(owned.toString().contains("iccProfile=["))
        val invalid = listOf(owned.copy(binding=source.binding.copy(hostSeq=8uL)),owned.copy(sourceAssetId="x".repeat(64)),
            owned.copy(width=0u),owned.copy(sourceBitDepth=32u),owned.copy(rgba=ByteArray(23)),
            owned.copy(width=10000u,height=5001u),owned.copy(width=UInt.MAX_VALUE,height=UInt.MAX_VALUE))
        invalid.forEach { value ->
            try { value.ownedImageEditingBackground(source.binding,ULong.MAX_VALUE); fail("Invalid composed background admitted") }
            catch (_: WorkflowFailure) { }
        }
        try { owned.ownedImageEditingBackground(source.binding,25uL); fail("Composed background exceeded byte budget") }
        catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Memory,failure.kind) }
    }

    @Test fun frameSetRejectsOversizedSourceBeforeIdsWithoutRestrictingUnframedLegacySource() {
        var ids = 0
        for ((width,height) in listOf(32769u to 1u,1u to 32769u)) {
            val source = snapshot().copy(sourceWidth=width,sourceHeight=height).ownedImageFramesSnapshot()
            assertNull(source.frame)
            val form = source.form().copy(cropWidth="1",cropHeight="1",outputWidth="1",outputHeight="1")
            try { form.command(source) { ids++; id(3) to id(4) }; fail("Oversized source grid admitted for a frame") }
            catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Invalid,failure.kind) }
            val background = ImageEditingBackground(source.binding,source.sourceAssetId,8u,width,height,
                ByteArray((width*height*4u).toInt()),ByteArray(0))
            assertEquals(width,background.ownedImageEditingBackground(source.binding,256uL*1024uL*1024uL).width)
        }
        assertEquals(0,ids)
    }

    private fun snapshot(frame: ImageFrameEntry? = null): ImageFramesSnapshot =
        ImageFramesSnapshot(WorkflowBinding(id(1),id(2),0uL,"a".repeat(64)),"c".repeat(64),1024u,768u,8u,frame)
    private fun id(number: Int): String = "00000000-0000-4000-8000-"+number.toString().padStart(12,'0')
}
