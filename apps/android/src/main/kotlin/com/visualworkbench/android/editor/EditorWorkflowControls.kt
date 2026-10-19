package com.visualworkbench.android.editor

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import com.visualworkbench.android.offline.OfflineReviewPanel

@Composable internal fun EditorWorkflowControls(editor:EditorController){
    val owner=editor.workflows
    val panel by owner.panel.collectAsState()
    val review by owner.review.state.collectAsState()
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(8.dp)){
        DocumentNavigatorPanel(owner.navigator,{owner.activate(EditorWorkflowPanel.Documents,it)},owner::choose,owner::cancelSwitch)
        LocalViewsPanel(owner.views,{owner.activate(EditorWorkflowPanel.SavedViews,it)},owner::saveView,owner::restoreView)
        LayersPanel(owner.controls,editor::workflowId,setOfNotNull(editor.selected),{owner.activate(EditorWorkflowPanel.Layers,it)})
        ToneControlsPanel(owner.tones,{owner.activate(EditorWorkflowPanel.Tones,it)})
        if(editor.frameControlsAvailable) ImageFramesPanel(editor.imageFrames,editor::framePanel)
        else OutlinedButton({},Modifier.heightIn(min=48.dp),enabled=false){Text("Output frame unavailable for this source")}
        OutlinedButton({owner.activate(EditorWorkflowPanel.OfflineReview,true)},Modifier.heightIn(min=48.dp),enabled=editor.document!=null){Text("Offline conflicts")}
    }
    if(editor.selected!=null)RotationSnapControl(editor.rotationSnap15,!editor.busy,editor::rotationSnap)
    if(panel==EditorWorkflowPanel.OfflineReview)AlertDialog(
        onDismissRequest={owner.activate(EditorWorkflowPanel.OfflineReview,false)},
        title={Text("Review offline conflicts")},
        text={OfflineReviewPanel(review,editor.document?.documentId,editor.document?.title?:"No document selected",
            owner.review::refresh,owner.review::firstPage,owner.review::nextPage,owner.review::select,
            owner.review::stageSwap,owner.review::stageRestore,owner.review::submit,owner.review::keep,
            owner.review::cancelDraft,owner.review::cancelOperation,Modifier.heightIn(max=560.dp))},
        confirmButton={TextButton({owner.activate(EditorWorkflowPanel.OfflineReview,false)},Modifier.heightIn(min=48.dp)){Text("Close")}})
}
