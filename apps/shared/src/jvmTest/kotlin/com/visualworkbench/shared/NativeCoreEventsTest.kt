package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertTrue

class NativeCoreEventsTest {
    @Test fun theRealCoreReturnsAnOrderedLogThatCanBeReadIncrementally() {
        val all = readCoreEvents()
        assertTrue(all.events.zipWithNext().all { (before, after) -> before.sequence < after.sequence })
        val newest = all.events.lastOrNull()?.sequence ?: 0L
        assertTrue(readCoreEvents(newest).events.all { it.sequence > newest })
        assertTrue(all.reportLines().size == all.events.size)
    }
}
