package com.visualworkbench.android.editor

import com.visualworkbench.android.session.RefreshAdmission
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
    /** Stands in for the editor around one fence: `pending` as the editor
     * counts it, the document it holds, and how often it read and adopted. */
    private class Editor {
        var pending=0;var reads=0;var adoptions=0;var held="old";var failures=0
        val admission=RefreshAdmission{pending+=it}
        val ready:Boolean get()=pending==0
        /** A save's own re-read, as the editor makes it while the save is pending. */
        suspend fun ownRead(result:String,during:suspend()->Unit={}):RefreshAdmission.Verdict=
            admission.refresh(queued=false,read={reads++;during();result},adopt={adoptions++;held=it})
        /** One turn of the queued-refresh loop. */
        suspend fun queuedTurn(result:String?,adopting:()->Unit={},during:suspend()->Unit={}){
            admission.queuedTurn({failures++}){admission.refresh(queued=true,read={reads++;during();result},adopt={adoptions++;held=it;adopting()})}
        }
    }
    /** The store announces a save from its own thread. When that announcement
     * lands during the save's own re-read, the read is dropped as it always
     * was and the queued refresh adopts. The save stays pending until then, so
     * the editor never looks ready while it still holds the old document. */
    @Test fun aSaveWhoseOwnReadIsOvertakenStaysPendingUntilTheQueuedRefreshAdopts():Unit=runBlocking {
        val editor=Editor();editor.admission.reset();editor.admission.observe(3u);val serving=editor.admission.serve()
        editor.pending++ // The save, counted by its caller for as long as it runs.
        val own=editor.ownRead("with-save"){assertTrue(editor.admission.observe(4u))} // Its change event lands during the read and queues a refresh.
        assertEquals(RefreshAdmission.Verdict.Overtaken,own);assertEquals(1,editor.reads);assertEquals(0,editor.adoptions)
        editor.pending-- // The caller is done. Before this change the editor looked ready here, with the old document.
        assertFalse(editor.ready);assertEquals("old",editor.held);assertEquals(1,editor.admission.owed)
        editor.queuedTurn("with-save",adopting={assertFalse(editor.ready)}) // Still pending while the document is being adopted.
        assertTrue(editor.ready);assertEquals("with-save",editor.held);assertEquals(0,editor.admission.owed)
        // Exactly what it cost before: two reads, one adoption, and the own read was not repeated.
        assertEquals(2,editor.reads);assertEquals(1,editor.adoptions);assertEquals(0,editor.failures)
        editor.admission.stopServing(serving);assertTrue(editor.ready)
    }
    @Test fun anAnnouncementBeforeOrAfterTheOwnReadCostsWhatItAlwaysDidAndHandsNothingOver():Unit=runBlocking {
        // Announced before the save's own read takes its ticket: that read adopts, the queued one adopts again.
        val early=Editor();early.admission.reset();early.admission.serve()
        early.pending++;assertTrue(early.admission.observe(1u))
        assertEquals(RefreshAdmission.Verdict.Adopt,early.ownRead("with-save"));early.pending--
        assertTrue(early.ready);assertEquals("with-save",early.held)
        early.queuedTurn("with-save")
        assertEquals(2,early.reads);assertEquals(2,early.adoptions);assertTrue(early.ready)
        // Announced after the save's own read was adopted.
        val late=Editor();late.admission.reset();late.admission.serve()
        late.pending++
        assertEquals(RefreshAdmission.Verdict.Adopt,late.ownRead("with-save"));late.pending--
        assertTrue(late.ready);assertTrue(late.admission.observe(1u))
        late.queuedTurn("with-save")
        assertEquals(2,late.reads);assertEquals(2,late.adoptions);assertTrue(late.ready);assertEquals(0,late.admission.owed)
    }
    @Test fun aSaveIsHandedOverOnlyWhileAQueuedRefreshIsStillGoingToRead():Unit=runBlocking {
        // The queued refresh for the announcement failed before it could read (the
        // project could not be asked) while the own read was still in flight: nobody
        // will adopt, so nothing is handed over and the editor is not left pending.
        val failed=Editor();failed.admission.reset();failed.admission.serve();failed.pending++
        val own=failed.ownRead("with-save"){
            assertTrue(failed.admission.observe(1u))
            failed.admission.queuedTurn({failed.failures++}){throw IllegalStateException("project unavailable")}
        }
        assertEquals(RefreshAdmission.Verdict.Overtaken,own);failed.pending--
        assertEquals(1,failed.failures);assertEquals(0,failed.admission.owed);assertTrue(failed.ready)
        // No queued-refresh loop at all, or one that has stopped.
        val idle=Editor();idle.admission.reset();idle.pending++
        assertEquals(RefreshAdmission.Verdict.Overtaken,idle.ownRead("x"){idle.admission.observe(1u)});idle.pending--
        assertTrue(idle.ready)
        val stopped=Editor();stopped.admission.reset();val token=stopped.admission.serve();stopped.admission.stopServing(token);stopped.pending++
        assertEquals(RefreshAdmission.Verdict.Overtaken,stopped.ownRead("x"){stopped.admission.observe(1u)});stopped.pending--
        assertTrue(stopped.ready)
        // The queued refresh is itself overtaken: it never hands itself over.
        val queued=Editor();queued.admission.reset();queued.admission.serve()
        queued.queuedTurn("x"){queued.admission.observe(1u)}
        assertEquals(0,queued.admission.owed);assertTrue(queued.ready);assertEquals(0,queued.adoptions)
    }
    @Test fun handedOverSavesAreSettledByEveryEndOfTheQueuedRefresh():Unit=runBlocking {
        suspend fun owing(saves:Int=1):Pair<Editor,Int>{
            val editor=Editor();editor.admission.reset();val serving=editor.admission.serve()
            repeat(saves){index->editor.pending++;editor.ownRead("save-$index"){editor.admission.observe((index+1).toULong())};editor.pending--}
            assertEquals(saves,editor.admission.owed);assertEquals(saves,editor.pending)
            return editor to serving
        }
        // An overtaken queued turn leaves them owed: the change that overtook it queued the next turn.
        val (overtaken)=owing()
        overtaken.queuedTurn("x"){overtaken.admission.observe(9u)}
        assertEquals(1,overtaken.admission.owed);assertFalse(overtaken.ready)
        overtaken.queuedTurn("y");assertTrue(overtaken.ready);assertEquals("y",overtaken.held)
        // Two saves handed over are settled together.
        val (two)=owing(2);two.queuedTurn("both");assertTrue(two.ready);assertEquals(0,two.admission.owed)
        // The page moved on during the queued read, or the read failed: nothing more will be adopted for them.
        val (moved)=owing();moved.queuedTurn(null);assertTrue(moved.ready)
        val (navigated)=owing();navigated.queuedTurn("x"){navigated.admission.invalidate()};assertTrue(navigated.ready);assertEquals("old",navigated.held)
        val (broken)=owing()
        broken.admission.queuedTurn({broken.failures++}){throw IllegalStateException("read failed")}
        assertTrue(broken.ready);assertEquals(1,broken.failures)
        // The loop or its observer stops; a token from an earlier loop does nothing.
        val (closing,serving)=owing()
        closing.admission.stopServing(serving-1);assertFalse(closing.ready)
        closing.admission.stopServing(serving);assertTrue(closing.ready)
        closing.admission.stopServing(serving);assertEquals(0,closing.pending)
    }
    @Test fun aReadAcrossNavigationOrAProjectChangeIsDroppedAndHandsNothingOver():Unit=runBlocking {
        val editor=Editor();editor.admission.reset();editor.admission.serve();editor.admission.observe(10u)
        assertEquals(RefreshAdmission.Verdict.Dropped,editor.ownRead("stale-page"){editor.admission.invalidate()})
        assertEquals(RefreshAdmission.Verdict.Dropped,editor.ownRead("stale-project"){editor.admission.reset()})
        // A read that reports its owner gone is dropped without a verdict on the fence.
        assertEquals(RefreshAdmission.Verdict.Dropped,editor.admission.refresh<String>(queued=false,read={null},adopt={fail()}))
        assertEquals(0,editor.admission.owed);assertEquals(0,editor.adoptions);assertTrue(editor.ready)
        val before=editor.admission.capture();editor.admission.observe(11u)
        assertTrue(editor.admission.overtaken(before));assertFalse(editor.admission.accepts(before))
        assertEquals(RefreshAdmission.Verdict.Overtaken,editor.admission.judge(before))
        editor.admission.invalidate();assertFalse(editor.admission.overtaken(before))
        assertEquals(RefreshAdmission.Verdict.Dropped,editor.admission.judge(before))
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
