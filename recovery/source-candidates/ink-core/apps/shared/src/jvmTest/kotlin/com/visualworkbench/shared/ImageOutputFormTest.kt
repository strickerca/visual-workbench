package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class ImageOutputFormTest {
    @Test fun quarterTurnsMapKnownDRectangleToOutputEdges() {
        val corners=listOf(Point(15.0,22.0),Point(25.0,22.0),Point(25.0,26.0),Point(15.0,26.0))
        val expected=listOf(Rect(25.0,10.0,50.0,20.0),Rect(70.0,25.0,20.0,50.0),Rect(125.0,70.0,50.0,20.0),Rect(10.0,125.0,20.0,50.0))
        listOf(0,90,180,270).forEachIndexed{index,angle->
            val odd=angle==90||angle==270
            assertEquals(expected[index],imageOutputRegion(snapshot(ImageOutputSettings(10u,20u,40u,20u,angle.toUShort(),if(odd)100u else 200u,if(odd)200u else 100u)),corners))
        }
    }
    @Test fun fractionalBoundsAreTransformedBeforeOutputRounding() {
        val value=snapshot(ImageOutputSettings(0u,0u,100u,80u,0u,1000u,800u))
        assertEquals(Rect(1.0,2.0,2.0,2.0),imageOutputRegion(value,listOf(Point(.1,.2),Point(.3,.2),Point(.3,.4),Point(.1,.4))))
    }
    @Test fun clippedOutsideSelectionRefusesInsteadOfFullImageFallback() {
        val value=snapshot(ImageOutputSettings(10u,20u,40u,20u,90u,100u,200u))
        refused { imageOutputRegion(value,listOf(Point(0.0,0.0),Point(1.0,0.0),Point(1.0,1.0),Point(0.0,1.0))) }
        refused { imageOutputRegion(value,List(5){Point(0.0,0.0)}) }
        refused { imageOutputRegion(value,List(4){Point(Double.NaN,0.0)}) }
    }
    @Test fun disabledFrameLeavesOriginalGridAndPreviewIsOneBoundedTile() {
        val value=snapshot(ImageOutputSettings(10u,20u,40u,20u,90u,1000u,2000u))
        assertEquals(SelectionRegion(990u,1990u,10u,10u),imageOutputPreviewRegion(value,990u,1990u))
        assertEquals(SelectionRegion(0u,0u,512u,512u),imageOutputPreviewRegion(value,0u,0u))
        refused{imageOutputPreviewRegion(value,1000u,0u)}
        val disabled=value.copy(output=value.output!!.copy(enabled=false))
        assertEquals(100u,disabled.outputWidth);assertEquals(80u,disabled.outputHeight)
    }
    @Test fun completeTextValidationPrecedesIdentitiesAndRejectsAmplification() {
        var ids=0;val next={ids++;if(ids%2==1)LAYER else OBJECT}
        val value=snapshot(null)
        val valid=value.form() as ImageOutputForm.Frame
        listOf(valid.copy(cropX="-1"),valid.copy(cropWidth="0"),valid.copy(outputWidth="4294967296"),
            valid.copy(outputWidth="50000"),valid.copy(outputWidth="10000",outputHeight="10000"),
            valid.copy(cropX="1".repeat(33)),valid.copy(clockwiseDegrees=45u)).forEach{refused{it.command(value,next)}}
        assertEquals(0,ids)
        val command=valid.command(value,next) as ImageOutputCommand.SetFrame
        assertEquals(2,ids);assertEquals(LAYER,command.layerId);assertEquals(OBJECT,command.objectId)
    }
    @Test fun savedFrameAndToneReuseExactObjectIdentities() {
        val value=snapshot(ImageOutputSettings(10u,20u,40u,20u,90u,100u,200u)).copy(tones=listOf(ImageToneEntry(LAYER,OBJECT,true,.5,false,ImageToneSettings(gamma=2.2))))
        val never={error("saved identity must not be replaced")}
        assertEquals(OBJECT,(value.form().command(value,never) as ImageOutputCommand.SetFrame).objectId)
        val tone=value.form(OBJECT,true) as ImageOutputForm.Tone
        assertEquals("2.2",tone.gamma);assertEquals(.5,(tone.command(value,never) as ImageOutputCommand.SetTone).opacity,0.0)
    }
    @Test fun unorderedOrNonfiniteToneValuesRefuseBeforeIds() {
        val value=snapshot(null);val form=ImageOutputForm.Tone(null)
        listOf(form.copy(gamma="NaN"),form.copy(gamma="0"),form.copy(inputBlack="0.6",inputWhite="0.5"),
            form.copy(outputBlack="0.9",outputWhite="0.1"),form.copy(opacity="1.1"),form.copy(contrast="Infinity"))
            .forEach{refused{it.command(value){error("must validate first")}}}
    }
    private fun snapshot(settings:ImageOutputSettings?)=ImageOutputSnapshot(WorkflowBinding(LAYER,OBJECT,1uL,"a".repeat(64)),"b".repeat(64),100u,80u,16u,
        settings?.let{ImageOutputEntry(LAYER,OBJECT,true,false,it)},emptyList())
    private fun refused(block:()->Unit){try{block();fail("Expected typed refusal")}catch(error:WorkflowFailure){assertEquals(WorkflowFailureKind.Invalid,error.kind)}}
    companion object {
        private const val LAYER="00000000-0000-4000-8000-000000000001"
        private const val OBJECT="00000000-0000-4000-8000-000000000002"
    }
}
