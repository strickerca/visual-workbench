package com.visualworkbench.shared

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Parent-process admission is separate from the isolated worker's own cap.
 * Four source/reply copies cover foreign serialization and validation buffers;
 * retained display ownership is charged before invoking the platform transport. */
public class BudgetedDocumentJobs(private val delegate:DocumentJobs,private val retainedBytes:()->Long,
    private val budgetBytes:Long=256L*1024*1024) : DocumentJobs {
    private val gate:Mutex=Mutex()
    public override val pcRedaction:Boolean get()=delegate.pcRedaction
    public override suspend fun run(request:DocumentRequest,source:ByteArray):DocumentReply = gate.withLock {
        val retained=retainedBytes()
        if(retained<0||retained>budgetBytes||source.isEmpty())throw DocumentFailure(DocumentFailureKind.Limit)
        val reply=when(val operation=request.operation){
            is DocumentOperation.Tile->{
                if(operation.tile.width !in 1u..512u||operation.tile.height !in 1u..512u||!operation.tile.scale.isFinite()||operation.tile.scale<=0||!operation.tile.origin.x.isFinite()||!operation.tile.origin.y.isFinite())throw DocumentFailure(DocumentFailureKind.Invalid)
                operation.tile.width.toLong()*operation.tile.height.toLong()*4
            }
            is DocumentOperation.Copy->documentOutputLimit(operation)
            else->0L
        }
        // Hard argument bounds precede multiplication/foreign copies.
        val svg=when(val op=request.operation){DocumentOperation.SvgInfo->true;is DocumentOperation.Tile->op.kind==EditableKind.Svg;is DocumentOperation.Copy->op.kind==DocumentCopy.SvgAnnotations;else->false}
        if(source.size>(if(svg)4 else 64)*1024*1024||reply<0||reply>128L*1024*1024)throw DocumentFailure(DocumentFailureKind.Limit)
        val peak=retained+source.size.toLong()*4+reply*4+36L*1024*1024
        if(peak>budgetBytes)throw DocumentFailure(DocumentFailureKind.Limit)
        delegate.run(request,source)
    }
    public override suspend fun close():Unit=delegate.close()
}
