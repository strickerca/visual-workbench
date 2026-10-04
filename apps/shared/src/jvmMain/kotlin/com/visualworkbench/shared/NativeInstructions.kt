package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.InstructionCommand as NCommand
import com.visualworkbench.bindings.core.InstructionRole as NRole
import com.visualworkbench.bindings.core.InstructionEntryMethod as NMethod
import com.visualworkbench.bindings.core.InstructionDraft as NDraft
import com.visualworkbench.bindings.core.DraftUpdateStatus as NUpdate
import com.visualworkbench.bindings.core.QueryRect
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.ObjectStyle as NStyle
import java.util.concurrent.atomic.AtomicBoolean

public actual fun instructionWorkflows(project:WorkbenchProject):WorkbenchInstructions = NativeInstructions(nativeProjectHandle(project))
private fun InstructionRole.native():NRole=when(this){InstructionRole.None->NRole.NONE;InstructionRole.Change->NRole.CHANGE;InstructionRole.Preserve->NRole.PRESERVE;InstructionRole.Reference->NRole.REFERENCE;InstructionRole.Explain->NRole.EXPLAIN}
private fun NRole.common():InstructionRole=when(this){NRole.NONE->InstructionRole.None;NRole.CHANGE->InstructionRole.Change;NRole.PRESERVE->InstructionRole.Preserve;NRole.REFERENCE->InstructionRole.Reference;NRole.EXPLAIN->InstructionRole.Explain}
private fun InstructionEntryMethod.native():NMethod=when(this){InstructionEntryMethod.PcKeyboard->NMethod.PC_KEYBOARD;InstructionEntryMethod.PhoneKeyboard->NMethod.PHONE_KEYBOARD;InstructionEntryMethod.Voice->NMethod.VOICE;InstructionEntryMethod.Handwriting->NMethod.HANDWRITING}
private fun NMethod.common():InstructionEntryMethod=when(this){NMethod.PC_KEYBOARD->InstructionEntryMethod.PcKeyboard;NMethod.PHONE_KEYBOARD->InstructionEntryMethod.PhoneKeyboard;NMethod.VOICE->InstructionEntryMethod.Voice;NMethod.HANDWRITING->InstructionEntryMethod.Handwriting}
private fun Rect.native():QueryRect=QueryRect(x,y,width,height)
private fun QueryRect.common():Rect=Rect(x,y,width,height)
private fun NPoint.common():Point=Point(x,y)
private fun Point.native():NPoint=NPoint(x,y)
private fun smallId(id:String){if(id.length!=36)throw WorkflowFailure(WorkflowFailureKind.Invalid)}
private fun checkText(text:String,language:String){if(text.length>32768 || language.length>128)throw WorkflowFailure(WorkflowFailureKind.Limit)}
internal fun InstructionCommand.native():NCommand=when(this){
    is InstructionCommand.PlaceMarker->{smallId(objectId);smallId(instructionId);smallId(layerId);checkText(text,language);if(elementEids.size>64 || elementEids.any{it.length>4096})throw WorkflowFailure(WorkflowFailureKind.Limit);NCommand.PlaceMarker(objectId,instructionId,layerId,point.native(),bounds?.native(),elementEids.toList(),NStyle(style.rgba,style.width,style.screenConstantWidth,style.fill),role.native(),text,entryMethod.native(),language)}
    is InstructionCommand.SetInstruction->{smallId(instructionId);checkText(text,language);if(targetIds.size>64)throw WorkflowFailure(WorkflowFailureKind.Limit);targetIds.forEach(::smallId);NCommand.SetInstruction(instructionId,targetIds.toList(),role.native(),text,entryMethod.native(),language)}
    is InstructionCommand.DeleteMarker->{smallId(objectId);NCommand.DeleteMarker(objectId)}
    is InstructionCommand.DeleteInstruction->{smallId(instructionId);NCommand.DeleteInstruction(instructionId)}
}
private fun closeDraft(value:NDraft){try{value.dispose()}finally{value.destroy()}}
private class NativeInstructions(private val handle:ProjectSession):WorkbenchInstructions {
    override suspend fun document(documentId:String,memoryBudgetBytes:ULong):InstructionDocument = settledWorkflow{cancel->
        val value=handle.instructionDocument(documentId,memoryBudgetBytes,cancel)
        InstructionDocument(workflowBinding(value.binding),value.instructions.map{InstructionRow(it.instructionId,it.targetIds,it.role.common(),it.text,it.entryMethod.common(),it.language,it.updatedAtMs,it.detached)},value.markers.map{MarkerRow(it.objectId,it.instructionId,it.number,it.pointDocument.common(),it.boundsDocument?.common(),it.elementEids,it.hidden,it.layerVisible)},value.needsReconciliation)
    }
    override suspend fun prepare(binding:WorkflowBinding,metadata:WorkflowMetadata,command:InstructionCommand,memoryBudgetBytes:ULong):WorkbenchWorkflowPlan {
        val request=command.native()
        val result=settledWorkflow(release=::closeWorkflowPlan){cancel->handle.prepareInstruction(workflowBinding(binding),workflowMetadata(metadata),request,memoryBudgetBytes,cancel)}
        return wrapWorkflowPlan(result)
    }
    override suspend fun beginDraft(binding:WorkflowBinding,instructionId:String,sessionId:String,focusGeneration:ULong,entryMethod:InstructionEntryMethod,memoryBudgetBytes:ULong):WorkbenchInstructionDraft {
        smallId(instructionId);smallId(sessionId)
        val result=settledWorkflow(release=::closeDraft){cancel->handle.beginInstructionDraft(workflowBinding(binding),instructionId,sessionId,focusGeneration,entryMethod.native(),memoryBudgetBytes,cancel)}
        try{return NativeInstructionDraft(result)}catch(error:Throwable){closeDraft(result);throw error}
    }
    override suspend fun export(binding:WorkflowBinding,memoryBudgetBytes:ULong):InstructionProjection = settledWorkflow{cancel->handle.exportInstructions(workflowBinding(binding),memoryBudgetBytes,cancel).let{InstructionProjection(workflowBinding(it.binding),it.json,it.promptFragment)}}
}
private class NativeInstructionDraft(private val handle:NDraft):WorkbenchInstructionDraft {
    private val closed=AtomicBoolean(false)
    private fun check(){if(closed.get())throw WorkflowFailure(WorkflowFailureKind.Closed)}
    override fun value():InstructionDraftValue=workflowDirect{check();handle.value().let{InstructionDraftValue(it.sessionId,it.instructionId,workflowBinding(it.binding),it.text,it.entryMethod.common())}}
    override fun update(sessionId:String,focusGeneration:ULong,sequence:ULong,text:String):DraftUpdateStatus=workflowDirect{
        check();smallId(sessionId);checkText(text,"")
        when(handle.update(sessionId,focusGeneration,sequence,text)){NUpdate.APPLIED->DraftUpdateStatus.Applied;NUpdate.DUPLICATE->DraftUpdateStatus.Duplicate;NUpdate.STALE->DraftUpdateStatus.Stale}
    }
    override suspend fun prepare(sessionId:String,focusGeneration:ULong,metadata:WorkflowMetadata):WorkbenchWorkflowPlan {
        check();smallId(sessionId)
        val result=settledWorkflow(release=::closeWorkflowPlan){cancel->handle.prepare(sessionId,focusGeneration,workflowMetadata(metadata),cancel)}
        return wrapWorkflowPlan(result)
    }
    override fun close(){if(!closed.getAndSet(true))workflowDirect{closeDraft(handle)}}
}
