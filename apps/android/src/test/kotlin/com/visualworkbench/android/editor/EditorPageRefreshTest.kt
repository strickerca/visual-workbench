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
    /** The store announces a save from its own thread. When that announcement
     * lands during the save's own re-read, the read is repeated and its newer
     * result adopted, instead of leaving the old document on screen. */
    @Test fun aReadOvertakenByItsOwnSaveEventIsRepeatedAndAdoptsTheNewerResult():Unit=runBlocking {
        val admission=com.visualworkbench.android.session.RefreshAdmission();admission.observe(3u)
        var reads=0
        val adopted=admission.readLatest(3){
            reads++
            if(reads==1){assertTrue(admission.observe(4u));"before-save"}else "after-save"
        }
        assertEquals("after-save",adopted);assertEquals(2,reads)
        // Nothing arrived during the read: exactly one read, as before.
        reads=0
        assertEquals("only",admission.readLatest(3){reads++;"only"});assertEquals(1,reads)
    }
    @Test fun aReadIsNeverRepeatedAcrossNavigationOrAProjectChangeAndStopsAtItsBound():Unit=runBlocking {
        val admission=com.visualworkbench.android.session.RefreshAdmission()
        var reads=0
        assertNull(admission.readLatest(3){reads++;admission.invalidate();"stale-page"});assertEquals(1,reads)
        reads=0
        assertNull(admission.readLatest(3){reads++;admission.reset();"stale-project"});assertEquals(1,reads)
        // A read that reports its owner gone ends the attempts without adoption.
        reads=0
        assertNull(admission.readLatest<String>(3){reads++;null});assertEquals(1,reads)
        // Events on every read: bounded, then left to the queued refresh as before.
        reads=0;var sequence=10uL
        assertNull(admission.readLatest(3){reads++;admission.observe(++sequence);"always-overtaken"});assertEquals(3,reads)
        val before=admission.capture();admission.observe(++sequence)
        assertTrue(admission.overtaken(before));assertFalse(admission.accepts(before))
        admission.invalidate();assertFalse(admission.overtaken(before))
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
