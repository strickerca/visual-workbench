@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import androidx.compose.ui.graphics.ImageBitmap
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
    private fun control(doc:DocumentSnapshot,vararg kinds:Pair<String,String>)=DocumentControlSnapshot(
        WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash),revision,
        emptyList(),emptyList(),kinds.map{(id,kind)->ObjectControlRow(id,"layer",null,"a",kind,false,false,Transform())},emptyList())

    @Test fun composedViewPaintsOnlyTransientItemsAndSelectionGeometryRemainsCanonical(){
        val doc=document();val binding=doc.binding(3L)
        val state=EditorState(document=doc,background=ImageBitmap(1,1),projectEpoch=3L,
            composedBackground=binding,selected=setOf("canonical"))
        assertTrue(composedCanvasCurrent(state,binding))
        assertFalse(composedCanvasCurrent(state.copy(background=null),binding))
        assertEquals(emptyList<RenderItem>(),canvasItems(doc,true,emptyMap()))
        assertEquals(listOf("transient"),canvasItems(doc,true,mapOf("transient" to item("transient"))).map{it.objectId})
        assertEquals(setOf("canonical"),state.selected)
    }

    @Test fun geometryMoveKeepsPhotoAndPreviewsOnlyAWireframe(){
        val doc=document();val binding=doc.binding(3L)
        val moved=item("canonical").copy(transform=Transform(e=12.0))
        val state=EditorState(document=doc,background=ImageBitmap(1,1),projectEpoch=3L,composedBackground=binding,
            preview=mapOf("canonical" to moved.transform))
        assertTrue(composedCanvasCurrent(state,binding))
        assertEquals(emptyList<RenderItem>(),canvasItems(doc,true,emptyMap()))
        assertEquals(listOf(Rect(12.0,0.0,8.0,8.0)),transientCanonicalOutlines(state,emptyMap()))
        assertTrue(composedCanvasCurrent(state.copy(preview=emptyMap(),previewStyles=mapOf("canonical" to moved.style)),binding))
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
        val peer=item("canonical").copy(bounds=Rect(9.0,0.0,8.0,8.0),transform=Transform(e=9.0))
        val state=EditorState(document=doc,background=ImageBitmap(1,1),projectEpoch=3L,composedBackground=binding,
            peerFrame=PeerFrame(binding,1L,PeerPreviews(1uL,emptyList(),listOf(peer))))
        assertTrue(composedCanvasCurrent(state,binding))
        assertEquals(emptyList<RenderItem>(),canvasItems(doc,true,mapOf("canonical" to peer)))
        assertEquals(listOf(peer.bounds),transientCanonicalOutlines(state,mapOf("canonical" to peer)))
    }

    @Test fun compositorGuidePassUsesTypedVectorSelectionAndNeverRepaintsOrdinaryPolygon(){
        val vector=item("vector",Shape.Polygon(listOf(Point(0.0,0.0),Point(8.0,0.0),Point(8.0,8.0)),true))
        val polygon=item("polygon",vector.shape)
        val guide=item("crop",Shape.Guide(Rect(0.0,0.0,8.0,8.0)))
        val doc=document().copy(render=RenderList(revision,listOf(item("canonical"),vector,polygon,guide,item("tone",Shape.Adjustment))))
        val binding=WorkflowBinding(revision.projectId,doc.documentId,revision.hostSeq,revision.stateHash)
        val ids=composedSelectionGuideIds(doc,control(doc,"vector" to "vector selection","polygon" to "polygon"),binding)
        assertEquals(setOf("vector"),ids)
        assertEquals(listOf("vector","crop"),canvasItems(doc,true,emptyMap(),ids).map{it.objectId})
        assertEquals(listOf("canonical","vector","polygon","crop"),canvasItems(doc,false,emptyMap(),ids).map{it.objectId})
        assertThrows(WorkflowFailure::class.java){composedSelectionGuideIds(doc,control(doc,"vector" to "polygon"),binding)}
        assertThrows(WorkflowFailure::class.java){composedSelectionGuideIds(doc,control(doc,"vector" to "vector selection","polygon" to "polygon"),binding.copy(stateHash="changed"))}
    }

    @Test fun typedVectorAndCropGuidesPrepareOnlyOutlines(){
        val vector=item("vector",Shape.Polygon(listOf(Point(0.0,0.0),Point(8.0,0.0),Point(8.0,8.0)),true))
            .copy(style=ObjectStyle(0xff0000ffu,0.0,fill=0xff0000ffu))
        assertEquals(listOf(PaintKind.Outline),prepareGeometry(vector,selectionGuide=true).passes.map{it.kind})
        assertTrue(prepareGeometry(vector).passes.any{it.kind==PaintKind.ObjectFill})
        val crop=item("crop",Shape.Guide(Rect(0.0,0.0,8.0,8.0))).copy(style=ObjectStyle(0xff0000ffu,0.0))
        assertEquals(listOf(PaintKind.Outline),prepareGeometry(crop).passes.map{it.kind})
    }
}
