package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import kotlin.coroutines.CoroutineContext

class McpOwnerWorkTest {
    private class Queued : CoroutineDispatcher() {
        private val tasks = ArrayDeque<Runnable>()
        override fun dispatch(context: CoroutineContext, block: Runnable) { tasks.addLast(block) }
        fun drain() { var count=0; while(tasks.isNotEmpty()) { check(++count<1000); tasks.removeFirst().run() } }
    }
    @Test fun cancelledBeforeBodyRepliesExactlyOnceAndRestoresAllFourSlots() {
        val dispatcher=Queued();val scope=CoroutineScope(SupervisorJob()+dispatcher)
        val replies=mutableListOf<Pair<String,Map<String,Any?>?>>()
        var bodies=0;var failed=0
        val owner=McpOwnerWork(scope,{id,result->replies += id to result},{failed++})
        try {
            repeat(4){i->assertTrue(owner.start("$i"){bodies++;mapOf("ok" to true)})}
            assertFalse(owner.start("full"){error("not admitted")})
            repeat(4){owner.cancel("$it")};dispatcher.drain()
            assertEquals(0,bodies);assertEquals(0,owner.active);assertEquals(4,replies.size)
            assertEquals((0..3).map{it.toString()}.toSet(),replies.map{it.first}.toSet());assertTrue(replies.all{it.second==null})
            repeat(4){i->assertTrue(owner.start("retry$i"){bodies++;mapOf("ok" to true)})};dispatcher.drain()
            assertEquals(4,bodies);assertEquals(0,owner.active);assertEquals(8,replies.size);assertEquals(0,failed)
        } finally {owner.seal().forEach{it.cancel()};scope.cancel();dispatcher.drain()}
    }
    @Test fun cancellationRetainsOwnershipUntilTheActualNativeProducerSettles() {
        val dispatcher=Queued();val scope=CoroutineScope(SupervisorJob()+dispatcher)
        val release=CompletableDeferred<Unit>();val replies=mutableListOf<Map<String,Any?>?>()
        val owner=McpOwnerWork(scope,{_,result->replies+=result},{error("reply failed")})
        try {
            assertTrue(owner.start("held"){withContext(NonCancellable){release.await()};mapOf("persisted" to true)})
            dispatcher.drain();owner.cancel("held");dispatcher.drain()
            assertEquals(1,owner.active);assertTrue(replies.isEmpty())
            val closing=owner.seal();assertEquals(1,closing.size);assertFalse(closing.single().isCompleted)
            release.complete(Unit);dispatcher.drain()
            assertTrue(closing.single().isCompleted);assertEquals(0,owner.active);assertEquals(1,replies.size);assertNull(replies.single())
            try {owner.start("late"){emptyMap()};fail("sealed ownership accepted work")} catch(_:McpRefused){}
        } finally {release.complete(Unit);owner.seal().forEach{it.cancel()};scope.cancel();dispatcher.drain()}
    }
    @Test fun anAlreadyCancelledParentAndAReplyFailureStillReleaseExactOwnership() {
        val dispatcher=Queued();val parent=SupervisorJob();val scope=CoroutineScope(parent+dispatcher)
        parent.cancel();var replies=0;var failed=0
        val owner=McpOwnerWork(scope,{_,_->replies++;throw McpRefused()},{failed++})
        try {assertTrue(owner.start("cancelled"){error("body must not execute")});dispatcher.drain()
            assertEquals(1,replies);assertEquals(1,failed);assertEquals(0,owner.active)
        } finally {owner.seal().forEach{it.cancel()};scope.cancel();dispatcher.drain()}
    }
}
