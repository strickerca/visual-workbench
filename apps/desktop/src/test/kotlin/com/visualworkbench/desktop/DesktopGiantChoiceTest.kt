@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class DesktopGiantChoiceTest {
    private val revision=ProjectInfo("project","image","owner",8u,true,false,5u,"c".repeat(64),listOf("document"))
    private val source="a".repeat(64)
    private val binding=WorkflowBinding("project","document",5u,"c".repeat(64))
    private val viewport=GiantViewport(0.0,0.0,256.0,256.0,1.0,0.0,0.0,1.0)
    private fun document(tones:Boolean)=DocumentSnapshot("document","image",100_000u,100_000u,8u,
        emptyList(),RenderList(revision,emptyList()),hasToneAdjustments=tones)
    private fun format(untagged:Boolean)=OriginalFormatInfo(binding,source,"png",8u,untagged,false)

    @Test fun taggedGiantSelectsNativeCompositionOnlyForSavedToneEdits(){
        val composed=giantDisplayChoice(document(true),format(false),null)
        assertEquals(GiantTileContent.Composed,composed?.content)
        assertFalse(checkNotNull(composed).assumeUntaggedSrgb)
        val raw=giantDisplayChoice(document(false),format(false),null)
        assertEquals(GiantTileContent.Source,raw?.content)
        assertFalse(checkNotNull(raw).assumeUntaggedSrgb)
    }

    @Test fun untaggedConsentIsExplicitAndBoundToTheExactSource(){
        assertNull(giantDisplayChoice(document(true),format(true),null))
        assertNull(giantDisplayChoice(document(true),format(true),"b".repeat(64)))
        assertTrue(checkNotNull(giantDisplayChoice(document(true),format(true),source)).assumeUntaggedSrgb)
        assertNull(giantDisplayChoice(document(true),format(false).copy(binding=binding.copy(hostSeq=6u)),source))
        assertNull(giantDisplayChoice(document(true),null,source))
    }

    @Test fun callerFrameFenceRequiresModeSourceRevisionEpochAndViewport(){
        val choice=checkNotNull(giantDisplayChoice(document(true),format(false),null))
        val frame=GiantVisibleFrame<Int>(binding,3L,9uL,viewport,source,"b".repeat(64),emptyList(),GiantTileContent.Composed)
        assertTrue(giantChoiceMatches(frame,choice,3L,viewport))
        assertFalse(giantChoiceMatches(frame.copy(content=GiantTileContent.Source),choice,3L,viewport))
        assertFalse(giantChoiceMatches(frame.copy(assetId="d".repeat(64)),choice,3L,viewport))
        assertFalse(giantChoiceMatches(frame.copy(binding=binding.copy(stateHash="d".repeat(64))),choice,3L,viewport))
        assertFalse(giantChoiceMatches(frame,choice,4L,viewport))
        assertFalse(giantChoiceMatches(frame,choice,3L,viewport.copy(left=1.0)))
    }

    @Test fun ownerChoiceChangesRetireTheOldModeAndColorContract(){
        val sourceChoice=checkNotNull(giantDisplayChoice(document(false),format(false),null))
        val composedChoice=checkNotNull(giantDisplayChoice(document(true),format(false),null))
        assertFalse(giantChoiceRequiresRetirement(sourceChoice,sourceChoice))
        assertTrue(giantChoiceRequiresRetirement(sourceChoice,sourceChoice,forceReplace=true))
        assertTrue(giantChoiceRequiresRetirement(sourceChoice,composedChoice))
        assertTrue(giantChoiceRequiresRetirement(composedChoice,composedChoice.copy(sourceAssetId="e".repeat(64))))
        assertTrue(giantChoiceRequiresRetirement(composedChoice,composedChoice.copy(assumeUntaggedSrgb=true)))
        assertTrue(giantChoiceRequiresRetirement(composedChoice,null))
    }

    @Test fun matchingComposedGiantSuppressesBakedObjectsButKeepsGuidesAndTransientPaint(){
        fun item(id:String,shape:Shape)=RenderItem(id,"layer",1.0,"normal",Rect(0.0,0.0,8.0,8.0),
            Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(),ObjectStyle(0xff0000ffu,1.0),shape,false)
        val canonical=item("canonical",Shape.Rectangle(Rect(0.0,0.0,8.0,8.0)))
        val guide=item("guide",Shape.Guide(Rect(0.0,0.0,8.0,8.0)))
        val transient=item("transient",Shape.Rectangle(Rect(10.0,0.0,8.0,8.0)))
        val result=item("result",Shape.Result("d".repeat(64)))
        val doc=document(true).copy(render=RenderList(revision,listOf(canonical,result,guide,item("tone",Shape.Adjustment))))
        val choice=checkNotNull(giantDisplayChoice(doc,format(false),null))
        val frame=GiantVisibleFrame<Int>(binding,3L,9uL,viewport,source,"b".repeat(64),emptyList(),GiantTileContent.Composed)
        assertTrue(giantChoiceMatches(frame,choice,3L,viewport))
        assertEquals(listOf("guide","transient"),canvasItems(doc,true,mapOf("transient" to transient)).map{it.objectId})
        assertEquals(listOf("canonical","result","guide"),canvasItems(doc,false,emptyMap()).map{it.objectId})
    }
}
