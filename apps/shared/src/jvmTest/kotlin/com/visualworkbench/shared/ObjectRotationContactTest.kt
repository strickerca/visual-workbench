@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class ObjectRotationContactTest {
    private val frame=ObjectRotationFrame(WorkflowBinding("project","document",1uL,"a".repeat(64)),2,
        Camera(Point(0.0,0.0),1.0,0.0,100.0,100.0))
    private val item=RenderItem("object","layer",1.0,"normal",Rect(10.0,20.0,20.0,10.0),
        Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(e=7.0),ObjectStyle(0x112233ffu,2.0),Shape.Rectangle(Rect(0.0,0.0,20.0,10.0)),false)
    private fun contact(items:List<RenderItem> = listOf(item), calls:MutableList<List<Transform>> = mutableListOf(),
        snaps:MutableList<Boolean> = mutableListOf()):ObjectRotationContact? = ObjectRotationContact.begin(frame,items,Point(20.0,25.0),Point(20.0,0.0)) { values,_,first,current,snap ->
            calls+=values.toList();snaps+=snap
            if(first==current)values else values.map{it.copy(e=if(snap)15.0 else current.x)}
        }

    @Test fun everyPreviewUsesTheSameOriginalAndCurrentSnapState() {
        val calls=mutableListOf<List<Transform>>();val snaps=mutableListOf<Boolean>();val c=contact(calls=calls,snaps=snaps)!!
        repeat(100){c.preview(Point(it.toDouble(),10.0),frame,it%2==0)}
        assertTrue(calls.all{it==listOf(item.transform)})
        assertEquals(false,snaps.last());assertEquals(101,calls.size)
    }
    @Test fun penUpUsesItsFinalRealPointAndProducesCommandsOnlyOnce() {
        val c=contact()!!;c.preview(Point(8.0,10.0),frame,true)
        val edits=c.finish(Point(19.0,10.0),frame,false)!!
        assertEquals(1,edits.size);assertEquals(19.0,(edits.single() as EditCommand.SetTransform).transform.e,0.0)
        assertNull(c.finish(Point(20.0,10.0),frame,true));assertNull(c.preview(Point(9.0,10.0),frame,false))
    }
    @Test fun everyAttachmentRevisionDocumentAndCameraChangeSealsBeforeACommit() {
        val changed=listOf(frame.copy(attachment=3),frame.copy(binding=frame.binding.copy(projectId="other")),
            frame.copy(binding=frame.binding.copy(documentId="other")),frame.copy(binding=frame.binding.copy(hostSeq=2uL)),
            frame.copy(binding=frame.binding.copy(stateHash="b".repeat(64))),frame.copy(camera=frame.camera.copy(scale=2.0)),frame.copy(page=1u))
        for(next in changed){val c=contact()!!;assertNull(c.preview(Point(30.0,10.0),next,false));assertNull(c.finish(null,frame,false))}
    }
    @Test fun cancellationAndUnchangedContactNeverCreateAnEdit() {
        val c=contact()!!;assertTrue(c.finish(null,frame,false)!!.isEmpty())
        val cancelled=contact()!!;cancelled.preview(Point(30.0,10.0),frame,false);cancelled.cancel();assertNull(cancelled.finish(null,frame,false))
    }
    @Test fun lockedGuidesAdjustmentsDuplicateTargetsAndOversizedGroupsRefuseBeforeSolver() {
        val calls=mutableListOf<List<Transform>>()
        for(items in listOf(emptyList(),listOf(item.copy(locked=true)),listOf(item.copy(shape=Shape.Guide(item.bounds))),
            listOf(item.copy(shape=Shape.Adjustment)),listOf(item,item),List(257){item.copy(objectId="item-$it")})){
            assertNull(contact(items,calls));assertTrue(calls.isEmpty())
        }
    }
    @Test fun solverFailureSealsTheContactWithoutKeepingACommit() {
        var n=0
        val c=ObjectRotationContact.begin(frame,listOf(item),Point(20.0,25.0),Point(20.0,0.0)){values,_,_,_,_->if(n++==0)values else throw CoreFailure(CoreFailureKind.Invalid)}!!
        try{c.preview(Point(30.0,10.0),frame,false);fail("invalid computation accepted")}catch(error:CoreFailure){assertEquals(CoreFailureKind.Invalid,error.kind)}
        assertNull(c.finish(null,frame,false))
    }
    @Test fun nonfiniteSamplesAndSolverOutputSealBeforeReturningAnyCommand() {
        val c=contact()!!
        try{c.preview(Point(Double.NaN,0.0),frame,false);fail("nonfinite point accepted")}catch(_:CoreFailure){}
        assertNull(c.finish(null,frame,false))
        var n=0
        val bad=ObjectRotationContact.begin(frame,listOf(item),Point(20.0,25.0),Point(20.0,0.0)){values,_,_,_,_->
            if(n++==0)values else values.map{it.copy(a=Double.POSITIVE_INFINITY)}}!!
        try{bad.preview(Point(10.0,10.0),frame,false);fail("nonfinite matrix accepted")}catch(_:CoreFailure){}
        assertNull(bad.finish(null,frame,false))
    }
}
