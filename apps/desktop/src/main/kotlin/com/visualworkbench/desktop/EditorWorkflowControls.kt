package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import com.visualworkbench.desktop.offline.OfflineReviewPanel

@Composable internal fun EditorWorkflowControls(controller:EditorController,editor:EditorState){
    val owner=controller.workflows
    val panel by owner.panel.collectAsState()
    val review by owner.review.state.collectAsState()
    DocumentNavigatorPanel(owner.navigator,{owner.activate(EditorWorkflowPanel.Documents,it)},owner::choose,owner::cancelSwitch,Modifier.fillMaxWidth())
    LocalViewsPanel(owner.views,{owner.activate(EditorWorkflowPanel.SavedViews,it)},owner::saveView,owner::restoreView,Modifier.fillMaxWidth())
    LayersPanel(owner.controls,controller::workflowId,editor.selected,{owner.activate(EditorWorkflowPanel.Layers,it)},Modifier.fillMaxWidth())
    ToneControlsPanel(owner.tones,{owner.activate(EditorWorkflowPanel.Tones,it)})
    if(editor.document?.page==null&&editor.document?.let(::giantDisplayRequired)==false)
        ImageFramesPanel(controller.frames,controller::activateFrames,Modifier.fillMaxWidth())
    OutlinedButton({owner.activate(EditorWorkflowPanel.OfflineReview,true)},Modifier.fillMaxWidth().heightIn(min=48.dp),enabled=editor.document!=null){Text("Offline conflicts")}
    if(panel==EditorWorkflowPanel.OfflineReview)AlertDialog(
        onDismissRequest={owner.activate(EditorWorkflowPanel.OfflineReview,false)},
        title={Text("Review offline conflicts")},
        text={OfflineReviewPanel(review,editor.document?.documentId,editor.document?.title?:"No document selected",
            owner.review::refresh,owner.review::firstPage,owner.review::nextPage,owner.review::select,
            owner.review::stageSwap,owner.review::stageRestore,owner.review::submit,owner.review::keep,
            owner.review::cancelDraft,owner.review::cancelOperation,Modifier.heightIn(max=640.dp))},
        confirmButton={TextButton({owner.activate(EditorWorkflowPanel.OfflineReview,false)}){Text("Close")}})
}
