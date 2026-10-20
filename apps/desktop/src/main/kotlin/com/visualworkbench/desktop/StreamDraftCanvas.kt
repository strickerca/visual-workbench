package com.visualworkbench.desktop

import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import com.visualworkbench.shared.Shape

/** The stream renderer owns every background pixel. A draft contributes only
 * annotation geometry, and cannot silently admit a second image owner. */
internal fun streamDraftOverlayValid(state:EditorState):Boolean = state.isStreamDraft&&
    state.document!=null&&state.background==null&&state.documentFrame==null&&
    state.composedBackground==null&&!state.giantActive&&!state.document.hasToneAdjustments&&
    state.document.render.items.none{it.shape is Shape.Result}

internal fun editorCanvasHasPixels(state:EditorState,giantReady:Boolean):Boolean =
    if(state.isStreamDraft)streamDraftOverlayValid(state)
    else state.background!=null||state.documentFrame!=null||state.composedBackground!=null||giantReady

internal fun DrawScope.drawEditorSurface(state:EditorState){
    if(!state.isStreamDraft)drawRect(Color(0xff0c1118))
}

internal fun DrawScope.drawEditorPage(state:EditorState,width:Float,height:Float){
    if(!state.isStreamDraft)drawRect(Color.White,size=Size(width,height))
}
