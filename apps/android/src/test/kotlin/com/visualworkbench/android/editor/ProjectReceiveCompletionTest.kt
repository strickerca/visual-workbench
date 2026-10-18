package com.visualworkbench.android.editor

import com.visualworkbench.shared.SessionFailureKind
import org.junit.Assert.*
import org.junit.Test

class ProjectReceiveCompletionTest {
    private data class Project(val id: String)

    @Test fun successfulCompletionCarriesTheExactProjectAndEpoch() {
        val received = Project("received")
        val completion = ProjectOpenCompletion(received, 7L)
        var connected: Project? = null
        var connectedEpoch: Long? = null
        completion.deliverIfCurrent(received, 7L) { project, epoch ->
            connected = project
            connectedEpoch = epoch
        }
        assertSame(received, connected)
        assertEquals(7L, connectedEpoch)
    }

    @Test fun lateCompletionCannotConnectAnEqualReplacementOrReopenedGeneration() {
        val received = Project("received")
        val replacement = Project("received")
        assertEquals(received, replacement)
        assertNotSame(received, replacement)
        val completion = ProjectOpenCompletion(received, 7L)
        var connects = 0
        completion.deliverIfCurrent(replacement, 7L) { _, _ -> connects++ }
        completion.deliverIfCurrent(received, 8L) { _, _ -> connects++ }
        completion.deliverIfCurrent(null, 7L) { _, _ -> connects++ }
        assertEquals(0, connects)
    }

    @Test fun receiveRefusalsRetainProjectContextAndNameTheRelevantRecoveryStep() {
        for (kind in SessionFailureKind.entries) {
            val message = projectReceiveFailureMessage(kind)
            assertTrue("Project context missing for $kind", message.contains("project", ignoreCase = true))
            assertFalse("Project receive described as code expiry for $kind", message.contains("pairing code", ignoreCase = true))
            assertFalse("Project storage described as trust write for $kind", message.contains("protected pairing store", ignoreCase = true))
        }
        for (kind in listOf(SessionFailureKind.Transport, SessionFailureKind.Timeout)) {
            assertTrue(projectReceiveFailureMessage(kind).contains("Use selected connection for USB project"))
            assertTrue(projectReceiveFailureMessage(kind).contains("Connect phone to this project"))
        }
        val storage = projectReceiveFailureMessage(SessionFailureKind.Storage)
        assertTrue(storage.contains("this phone"))
        assertTrue(storage.contains("saved projects are unchanged"))
        assertTrue(projectReceiveFailureMessage(SessionFailureKind.Authentication).contains("selected computer"))
    }
}
