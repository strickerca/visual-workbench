package com.visualworkbench.android.editor

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*

@Composable
internal fun LocalViewsPanel(views:LocalViewsInteraction,activate:(Boolean)->Unit,
    save:(LocalViewsBinding,String)->Unit,restore:(LocalViewsBinding,String)->Unit,modifier:Modifier=Modifier) {
    val state by views.state.collectAsState()
    OutlinedButton(onClick={activate(true)},modifier=modifier.heightIn(min=48.dp)) {
        Text(if(state.retryPending)"Saved views · retry needed" else "Saved views")
    }
    if(!state.active)return
    var name by remember(state.binding){mutableStateOf("")}
    val bound=state.binding
    val ready=state.available&&!state.loading&&!state.saving&&!state.retryPending&&bound!=null
    val valid=name.isNotBlank()&&name.trim()==name&&name.length<=256&&name.encodeToByteArray().size<=256&&name.none{it.isISOControl()}
    AlertDialog(onDismissRequest={activate(false)},title={Text("Saved views on this device")},text={
        LazyColumn(Modifier.fillMaxWidth().heightIn(min=200.dp,max=560.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            item {
                Text("Camera position, zoom and rotation stay on this device. Restoring uses the current window size. Originals and document history are unchanged.",style=MaterialTheme.typography.body2)
                if(state.loading||state.saving)LinearProgressIndicator(Modifier.fillMaxWidth())
                state.issue?.let{Text(it,style=MaterialTheme.typography.body2)}
                if(state.retryPending&&bound!=null) {
                    TextButton(onClick={views.retry(bound)},enabled=!state.saving,modifier=Modifier.heightIn(min=48.dp)){Text("Retry exact view change")}
                    TextButton(onClick={views.reloadDiscardingUnsavedView(bound)},enabled=!state.saving,modifier=Modifier.heightIn(min=48.dp)){Text("Reload saved views; discard unsaved view change")}
                }
                OutlinedTextField(name,{if(it.length<=256)name=it},label={Text("Name for new or renamed view")},enabled=!state.saving,modifier=Modifier.fillMaxWidth())
                TextButton(onClick={bound?.let{save(it,name)}},enabled=ready&&valid,modifier=Modifier.heightIn(min=48.dp)){Text("Save current view")}
            }
            items(state.bookmarks,key={it.id}) { bookmark ->
                Column(Modifier.fillMaxWidth()) {
                    Text(bookmark.name,style=MaterialTheme.typography.subtitle1)
                    Text("${bookmark.camera.scale}× · ${bookmark.camera.rotation*180.0/kotlin.math.PI}°",style=MaterialTheme.typography.caption)
                    Row {
                        TextButton(onClick={bound?.let{restore(it,bookmark.id)}},enabled=ready,modifier=Modifier.heightIn(min=48.dp)){Text("Restore")}
                        TextButton(onClick={bound?.let{views.renameBookmark(it,bookmark.id,name)}},enabled=ready&&valid,modifier=Modifier.heightIn(min=48.dp)){Text("Rename")}
                        TextButton(onClick={bound?.let{views.deleteBookmark(it,bookmark.id)}},enabled=ready,modifier=Modifier.heightIn(min=48.dp)){Text("Remove")}
                    }
                }
            }
        }
    },confirmButton={TextButton(onClick={activate(false)},modifier=Modifier.heightIn(min=48.dp)){Text("Close")}})
}
