package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.ProjectInfo
import com.visualworkbench.shared.RenderList
import org.junit.Assert.*
import org.junit.Test

class StreamDraftCanvasTest {
    @Test fun retainedSourceRemainsVisibleUnderTransparentAnnotation(){
        val source=Color(0xff224466)
        val state=draftCanvasState(true)
        val image=paint(state,source)
        assertEquals(source.toArgb(),image.toPixelMap()[0,0].toArgb())
        assertEquals(source.toArgb(),image.toPixelMap()[7,7].toArgb())
        assertEquals(Color.Red.toArgb(),image.toPixelMap()[2,2].toArgb())
    }

    @Test fun ordinaryPageAndSurfaceKeepTheirOpaquePaint(){
        val image=paint(draftCanvasState(false),Color(0xff224466))
        assertEquals(Color.White.toArgb(),image.toPixelMap()[0,0].toArgb())
        assertEquals(Color(0xff0c1118).toArgb(),image.toPixelMap()[7,7].toArgb())
        assertEquals(Color.Red.toArgb(),image.toPixelMap()[2,2].toArgb())
    }

    @Test fun draftAdmitsItsBorrowedBackgroundOnlyAndRefusesASecondPixelOwner(){
        val draft=draftCanvasState(true)
        assertTrue(editorCanvasHasPixels(draft,false))
        assertFalse(editorCanvasHasPixels(draft.copy(background=ImageBitmap(8,8)),false))
        assertFalse(editorCanvasHasPixels(draft.copy(document=null),false))
        assertFalse(editorCanvasHasPixels(draft.copy(isStreamDraft=false),false))
    }

    private fun paint(state:EditorState,source:Color):ImageBitmap {
        val image=ImageBitmap(8,8)
        CanvasDrawScope().draw(Density(1f),LayoutDirection.Ltr,Canvas(image),Size(8f,8f)){
            drawRect(source)
            drawEditorSurface(state)
            drawEditorPage(state,6f,6f)
            drawRect(Color.Red,topLeft=Offset(2f,2f),size=Size(2f,2f))
        }
        return image
    }
}

private fun draftCanvasState(draft:Boolean):EditorState {
    val revision=ProjectInfo(projectId="draft-project",title="draft",deviceId="synthetic-device",
        nextLamport=2uL,canUndo=false,canRedo=false,hostSeq=1uL,stateHash="a".repeat(64),
        documentIds=listOf("draft-document"))
    val document=DocumentSnapshot("draft-document","draft",8u,8u,8u,emptyList(),RenderList(revision,emptyList()))
    return EditorState(document=document,isStreamDraft=draft)
}
