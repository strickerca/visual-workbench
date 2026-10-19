@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class DesktopComposedCanvasTest {
    private val revision=ProjectInfo("project","image","owner",8u,true,false,5u,"hash-5",listOf("document"))
    private fun item(id:String,shape:Shape=Shape.Rectangle(Rect(0.0,0.0,8.0,8.0)))=RenderItem(
        id,"layer",1.0,"normal",Rect(0.0,0.0,8.0,8.0),Contours(longArrayOf(),longArrayOf(),uintArrayOf()),
        Transform(),ObjectStyle(0xff0000ffu,1.0),shape,false)
    private fun document()=DocumentSnapshot("document","image",64u,64u,8u,
        listOf(LayerInfo("layer","Annotations",true,false,1.0,"normal")),
        RenderList(revision,listOf(item("canonical"),item("tone",Shape.Adjustment))),hasToneAdjustments=true)

    @Test fun composedViewPaintsOnlyTransientItemsAndSelectionGeometryRemainsCanonical(){
        val doc=document();val binding=doc.binding(3L)
        val state=EditorState(document=doc,projectEpoch=3L,composedBackground=binding,selected=setOf("canonical"))
        assertTrue(composedCanvasCurrent(state,binding))
        assertEquals(emptyList<RenderItem>(),canvasItems(doc,true,emptyMap()))
        assertEquals(listOf("transient"),canvasItems(doc,true,mapOf("transient" to item("transient"))).map{it.objectId})
        assertEquals(setOf("canonical"),state.selected)
    }

    @Test fun geometryMoveRetiresCompositionAndPaintsCanonicalReplacementOnce(){
        val doc=document();val binding=doc.binding(3L)
        val moved=item("canonical").copy(transform=Transform(e=12.0))
        val state=EditorState(document=doc,projectEpoch=3L,composedBackground=binding,
            preview=mapOf("canonical" to moved.transform))
        assertFalse(composedCanvasCurrent(state,binding))
        assertEquals(listOf(moved),canvasItems(doc,false,mapOf("canonical" to moved)))
        assertFalse(composedCanvasCurrent(state.copy(preview=emptyMap(),previewStyles=mapOf("canonical" to moved.style)),binding))
        assertFalse(composedCanvasCurrent(state.copy(preview=emptyMap(),projectEpoch=4L),doc.binding(4L)))
    }

    @Test fun lastToneRemovalRestoresOrdinaryCanonicalItemsWithoutPaintingMetadata(){
        val doc=document().copy(hasToneAdjustments=false)
        val items=canvasItems(doc,false,emptyMap())
        assertEquals(listOf("canonical"),items.map{it.objectId})
        assertEquals(emptyList<RenderItem>(),canvasItems(doc,true,emptyMap()))
    }

    @Test fun peerReplacementCannotOverlayTheBakedCanonicalPose(){
        val doc=document();val binding=doc.binding(3L)
        val state=EditorState(document=doc,projectEpoch=3L,composedBackground=binding,
            peerFrame=PeerFrame(binding,1L,PeerPreviews(1uL,emptyList(),listOf(item("canonical")))))
        assertFalse(composedCanvasCurrent(state,binding))
        assertEquals(listOf("canonical"),canvasItems(doc,false,mapOf("canonical" to item("canonical"))).map{it.objectId})
    }
}
