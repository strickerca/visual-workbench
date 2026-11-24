package com.visualworkbench.android.editor

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import com.visualworkbench.ui.*

/** A failed or still-loading format query cannot hide the owner that guards edits. */
internal fun showFrameControls(available: Boolean, state: ImageFramesState): Boolean =
    available || state.blocksInput

@Composable internal fun EditorWorkflowControls(editor:EditorController){
    val owner=editor.workflows
    val panel by owner.panel.collectAsState()
    val review by owner.review.state.collectAsState()
    val frame by editor.imageFrames.state.collectAsState()
    val tone by owner.tones.state.collectAsState()
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(8.dp)){
        DocumentNavigatorPanel(owner.navigator,{owner.activate(EditorWorkflowPanel.Documents,it)},owner::choose,owner::cancelSwitch)
        LocalViewsPanel(owner.views,{owner.activate(EditorWorkflowPanel.SavedViews,it)},owner::saveView,owner::restoreView)
        LayersPanel(owner.controls,editor::workflowId,setOfNotNull(editor.selected),{owner.activate(EditorWorkflowPanel.Layers,it)})
        if(editor.toneControlsAvailable || tone.blocksInput) ToneControlsPanel(owner.tones,{owner.activate(EditorWorkflowPanel.Tones,it)},{TonePreviewTile(it)})
        else OutlinedButton({},Modifier.heightIn(min=48.dp),enabled=false){Text("Tone unavailable for this source")}
        if(showFrameControls(editor.frameControlsAvailable,frame)) ImageFramesPanel(editor.imageFrames,editor::framePanel)
        else OutlinedButton({},Modifier.heightIn(min=48.dp),enabled=false){Text("Output frame unavailable for this source")}
        OutlinedButton({owner.activate(EditorWorkflowPanel.OfflineReview,true)},Modifier.heightIn(min=48.dp),enabled=editor.document!=null){Text("Offline conflicts")}
    }
    // Shown for as long as the selection tool is (objects are turned only with
    // it), so that selecting or letting go of an object never changes the
    // canvas's size: that would make its whole picture again and end a drag
    // that is under way.
    if(editor.tool==EditorTool.Select)RotationSnapControl(editor.rotationSnap15,editor.selected!=null&&!editor.busy,editor::rotationSnap)
    if(panel==EditorWorkflowPanel.OfflineReview)AlertDialog(
        onDismissRequest={owner.activate(EditorWorkflowPanel.OfflineReview,false)},
        title={Text("Review offline conflicts")},
        text={OfflineReviewPanel(review,editor.document?.documentId,editor.document?.title?:"No document selected",
            owner.review::refresh,owner.review::firstPage,owner.review::nextPage,owner.review::select,
            owner.review::stageSwap,owner.review::stageRestore,owner.review::submit,owner.review::keep,
            owner.review::cancelDraft,owner.review::cancelOperation,Modifier.heightIn(max=560.dp))},
        confirmButton={TextButton({owner.activate(EditorWorkflowPanel.OfflineReview,false)},Modifier.heightIn(min=48.dp)){Text("Close")}})
}
