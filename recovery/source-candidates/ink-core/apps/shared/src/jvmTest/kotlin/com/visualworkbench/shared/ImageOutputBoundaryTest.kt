package com.visualworkbench.shared

import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

public class ImageOutputBoundaryTest {
    private val layer = "00000000-0000-7000-8000-000000000001"
    private val obj = "00000000-0000-7000-8000-000000000002"
    private fun settings(): ImageOutputSettings = ImageOutputSettings(2u,3u,20u,30u,90u,90u,40u)
    private fun refused(block: () -> Unit) {
        try { block(); fail("invalid output must be refused before crossing the native boundary") }
        catch (failure: WorkflowFailure) { assertEquals(WorkflowFailureKind.Invalid,failure.kind) }
    }
    @Test public fun explicitIndependentAxisResizeKeepsOwnerDimensionsAndQuarterTurn() {
        val value = ImageOutputCommand.SetFrame(layer,obj,settings(),true)
        value.validateImageOutput()
        assertEquals(90u,value.settings.outputWidth);assertEquals(40u,value.settings.outputHeight)
        assertEquals(90.toUShort(),value.settings.clockwiseDegrees)
    }
    @Test public fun angleDepthAndExtentPolicyNeverSilentlyClampsOrResizes() {
        for (setting in listOf(settings().copy(clockwiseDegrees=45u),settings().copy(outputWidth=0u),
            settings().copy(outputHeight=32769u),settings().copy(outputWidth=32768u,outputHeight=32768u))) {
            refused { ImageOutputCommand.SetFrame(layer,obj,setting,true).validateImageOutput() }
        }
    }
    @Test public fun levelsMustBeFiniteOrderedAndHavePositiveGamma() {
        for (tone in listOf(ImageToneSettings(gamma=0.0),ImageToneSettings(inputBlack=1.0),
            ImageToneSettings(outputBlack=0.8,outputWhite=0.2),ImageToneSettings(contrast=Double.NaN))) {
            refused { ImageOutputCommand.SetTone(layer,obj,tone,true,1.0).validateImageOutput() }
        }
        ImageOutputCommand.SetTone(layer,obj,ImageToneSettings(gamma=1.4),false,0.5).validateImageOutput()
    }
    @Test public fun frameAndToneIdentityMustRemainDistinctForTheAtomicPlan() {
        refused { ImageOutputCommand.SetFrame(layer,layer,settings(),true).validateImageOutput() }
        refused { ImageOutputCommand.RemoveTone(layer,"not-an-object").validateImageOutput() }
        refused { ImageOutputCommand.SetTone(layer,obj,ImageToneSettings(),true,Double.POSITIVE_INFINITY).validateImageOutput() }
    }
}
