package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Runs the same suspended read/adoption boundary used by this app's root
 * refresh path. No framework/native parser or UI event delivery is simulated. */
class EditorPageRefreshTest {
    @Test fun navigationInvalidatesAndroidRefreshTicketsWithoutRewindingEvents() {
        val admission=com.visualworkbench.android.session.RefreshAdmission()
        assertTrue(admission.observe(7u));val before=admission.capture();admission.invalidate()
        assertFalse(admission.accepts(before));assertEquals(7uL,admission.capture().sequence)
        assertFalse(admission.observe(7u));assertTrue(admission.observe(8u))
    }
    private val revision=ProjectInfo("project","title","device",1u,false,false,7u,"same-revision",listOf("doc"))
    private fun page(index:UInt):DocumentSnapshot=DocumentSnapshot("doc","title",64u,64u,8u,
        listOf(LayerInfo("layer-$index","page",true,false,1.0,"normal")),RenderList(revision,emptyList()),index)
    @Test fun acquiredOldPageCannotAdoptOrRebindAfterNavigation():Unit=runBlocking {
        val admission=DocumentRefreshAdmission();var current=page(0u);val acquired=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        var adopted:DocumentSnapshot?=null;var toolBinding:DocumentSnapshot?=null
        val refresh=launch(start=CoroutineStart.UNDISPATCHED){
            val loaded=admission.load(current,{current}){val captured=current;acquired.complete(Unit);release.await();captured}
            if(loaded!=null&&admission.accepts(loaded.ticket,current,loaded.snapshot)){adopted=loaded.snapshot;toolBinding=loaded.snapshot}
        }
        acquired.await();admission.invalidate();current=page(1u);release.complete(Unit);refresh.join()
        assertNull(adopted);assertNull(toolBinding);assertEquals(1u,current.page)
        val loaded=requireNotNull(admission.load(current,{current}){current})
        assertTrue(admission.accepts(loaded.ticket,current,loaded.snapshot))
    }
    @Test fun pageReturnAndPostReadSuspensionCannotRevivePriorOwnership():Unit=runBlocking {
        val admission=DocumentRefreshAdmission();var current=page(0u)
        val loaded=requireNotNull(admission.load(current,{current}){current})
        admission.invalidate();current=page(1u)
        assertFalse(admission.accepts(loaded.ticket,current,loaded.snapshot))
        admission.invalidate();current=page(0u)
        assertFalse(admission.accepts(loaded.ticket,current,loaded.snapshot))
        assertNull(admission.load(current,{current}){page(1u)})
    }
}
