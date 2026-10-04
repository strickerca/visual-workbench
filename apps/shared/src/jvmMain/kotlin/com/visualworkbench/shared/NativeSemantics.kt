package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.SemanticPlatform as NPlatform
import com.visualworkbench.bindings.core.SemanticBoundsSpace as NSpace
import com.visualworkbench.bindings.core.SemanticCollection as NCollection
import com.visualworkbench.bindings.core.CapturedSemanticElement as NElement
import com.visualworkbench.bindings.core.SemanticSnapQuery as NQuery
import com.visualworkbench.bindings.core.SemanticCaptureInfo as NCapture
import com.visualworkbench.bindings.core.QueryRect
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.Transform as NTransform
import java.util.concurrent.atomic.AtomicBoolean

public actual fun semanticWorkflows(project:WorkbenchProject):WorkbenchSemantics=NativeSemantics(nativeProjectHandle(project))
private fun SemanticPlatform.native():NPlatform=when(this){SemanticPlatform.Uia->NPlatform.UIA;SemanticPlatform.AndroidAx->NPlatform.ANDROID_AX;SemanticPlatform.ChromiumUia->NPlatform.CHROMIUM_UIA}
private fun NPlatform.common():SemanticPlatform=when(this){NPlatform.UIA->SemanticPlatform.Uia;NPlatform.ANDROID_AX->SemanticPlatform.AndroidAx;NPlatform.CHROMIUM_UIA->SemanticPlatform.ChromiumUia}
private fun SemanticBoundsSpace.native():NSpace=when(this){SemanticBoundsSpace.HostPhysical->NSpace.HOST_PHYSICAL;SemanticBoundsSpace.CapturePixels->NSpace.CAPTURE_PIXELS}
private fun Rect.native():QueryRect=QueryRect(x,y,width,height)
private fun QueryRect.common():Rect=Rect(x,y,width,height)
private fun NCapture.common():SemanticCaptureInfo=SemanticCaptureInfo(workflowBinding(binding),captureSessionId,frameId,geometryRevision,sourceAssetId,capturedAtMs,width,height)
private fun smallSnapshot(id:String){if(id.length!=36)throw WorkflowFailure(WorkflowFailureKind.Invalid)}
private fun admitElements(elements:List<CapturedSemanticElement>){
    if(elements.size>4096)throw WorkflowFailure(WorkflowFailureKind.Limit)
    var total=0L
    for(element in elements){
        for((text,limit) in listOf(element.localId to 256,element.parentLocalId to 256,element.name to 4096,element.role to 256,element.automationId to 1024,element.resourceId to 1024,element.htmlId to 1024,element.text to 800)){
            if(text!=null){if(text.length>limit)throw WorkflowFailure(WorkflowFailureKind.Limit);total+=text.length;if(total>2*1024*1024)throw WorkflowFailure(WorkflowFailureKind.Limit)}
        }
    }
}
private fun CapturedSemanticElement.native():NElement=NElement(localId,parentLocalId,name,role,automationId,resourceId,htmlId,bounds.native(),text,enabled,focused)
private fun closeCollection(value:NCollection){try{value.dispose()}finally{value.destroy()}}
private class NativeSemantics(private val handle:ProjectSession):WorkbenchSemantics {
    override suspend fun beginCapture(binding:WorkflowBinding,memoryBudgetBytes:ULong):WorkbenchSemanticCollection{
        val nativeBinding=workflowBinding(binding)
        val result=settledWorkflow(release=::closeCollection){cancel->handle.beginSemanticCapture(nativeBinding,memoryBudgetBytes,cancel)}
        try{return NativeSemanticCollection(result)}catch(error:Throwable){closeCollection(result);throw error}
    }
    override suspend fun document(binding:WorkflowBinding,snapshotId:String,memoryBudgetBytes:ULong):SemanticDocument{
        smallSnapshot(snapshotId);val nativeBinding=workflowBinding(binding)
        return settledWorkflow{cancel->handle.semanticDocument(nativeBinding,snapshotId,memoryBudgetBytes,cancel).let{value->SemanticDocument(workflowBinding(value.binding),value.snapshotId,value.platform.common(),value.frameDeltaMs,value.collectionElapsedMs,value.elements.map{SemanticElement(it.eid,it.parent,it.name,it.role,it.automationId,it.resourceId,it.htmlId,it.boundsDocument.common(),it.boundsClipped,it.text,it.enabled,it.focused)})}}
    }
    override suspend fun snap(binding:WorkflowBinding,snapshotId:String,query:SemanticSnapQuery,documentToScreen:Transform,memoryBudgetBytes:ULong):SemanticSnap?{
        smallSnapshot(snapshotId);val nativeBinding=workflowBinding(binding)
        val request=when(query){is SemanticSnapQuery.PointQuery->NQuery.Point(NPoint(query.point.x,query.point.y));is SemanticSnapQuery.BoxQuery->NQuery.Box(query.bounds.native())}
        val m=documentToScreen
        return settledWorkflow{cancel->handle.snapSemantic(nativeBinding,snapshotId,request,NTransform(m.a,m.b,m.c,m.d,m.e,m.f),memoryBudgetBytes,cancel)?.let{SemanticSnap(it.snapshotId,workflowBinding(it.binding),it.eid,it.boundsDocument.common(),it.distanceScreenPixels)}}
    }
    override suspend fun export(binding:WorkflowBinding,snapshotId:String,elementEids:List<String>?,memoryBudgetBytes:ULong):SemanticProjection{
        smallSnapshot(snapshotId);val nativeBinding=workflowBinding(binding)
        if(elementEids!=null && (elementEids.size>64 || elementEids.any{it.length>300}))throw WorkflowFailure(WorkflowFailureKind.Limit)
        val ids=elementEids?.toList()
        return settledWorkflow{cancel->handle.exportSemantics(nativeBinding,snapshotId,ids,memoryBudgetBytes,cancel).let{value->SemanticProjection(workflowBinding(value.binding),value.snapshotId,value.references.map{SemanticReference(it.platform.common(),it.eid,it.name,it.role,it.automationId,it.resourceId,it.htmlId,it.boundsDocument.common())},value.semanticJson,value.quotedPromptData)}}
    }
}
private class NativeSemanticCollection(private val handle:NCollection):WorkbenchSemanticCollection{
    private val closed=AtomicBoolean(false)
    private fun check(){if(closed.get())throw WorkflowFailure(WorkflowFailureKind.Closed)}
    override fun describe():SemanticCaptureInfo=workflowDirect{check();handle.describe().common()}
    override suspend fun prepare(metadata:WorkflowMetadata,snapshotId:String,platform:SemanticPlatform,frameDeltaMs:Int,collectionElapsedMs:ULong,boundsSpace:SemanticBoundsSpace,elements:List<CapturedSemanticElement>):WorkbenchWorkflowPlan{
        check();smallSnapshot(snapshotId);admitElements(elements);val rows=elements.map{it.native()};val meta=workflowMetadata(metadata)
        val result=settledWorkflow(release=::closeWorkflowPlan){cancel->handle.prepare(meta,snapshotId,platform.native(),frameDeltaMs,collectionElapsedMs,boundsSpace.native(),rows,cancel)}
        return wrapWorkflowPlan(result)
    }
    override fun close(){if(!closed.getAndSet(true))workflowDirect{closeCollection(handle)}}
}
