package com.visualworkbench.android.assistedselection

import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class AssistedSourceGateTest {
    private val revision=ProjectInfo("same-project-id","image","device",1uL,false,false,4uL,"a".repeat(64),listOf("document"))
    private val source=AssistedSelectionSource(SelectionDocument(
        SelectionBinding("same-project-id","document","b".repeat(64),4uL,"a".repeat(64)),
        16u,16u,8u,revision),null,2L)

    @Test fun projectReferenceRevisionPageAndEpochCannotBeReboundByAnOldWorker() {
        val gate=AssistedSourceGate<Any>()
        val first=Any();val reopened=Any()
        gate.publish(first,source)
        assertEquals(source,gate.sourceFor(first));assertTrue(gate.matches(first,source))
        assertNull(gate.sourceFor(reopened));assertFalse(gate.matches(reopened,source))
        for(stale in listOf(source.copy(attachmentEpoch=3L),source.copy(page=1u),
            source.copy(document=source.document.copy(binding=source.binding.copy(hostSeq=5uL)))))
            assertFalse(gate.matches(first,stale))
        gate.revoke();assertNull(gate.sourceFor(first))
        gate.publish(reopened,source);assertNull(gate.sourceFor(first))
    }

    @Test fun staleOwnerRetiresBeforeFreshIdleAdmissionAndFailedCloseKeepsOwner() = runBlocking {
        assertFalse(reusableAssistedOwner(false,AssistedSelectionPhase.Ready))
        assertFalse(reusableAssistedOwner(true,AssistedSelectionPhase.Stale))
        assertFalse(reusableAssistedOwner(true,AssistedSelectionPhase.Failed))
        assertTrue(reusableAssistedOwner(true,AssistedSelectionPhase.Ready))
        val stale=Any()
        var retained: Any?=stale
        val order=mutableListOf<String>()
        val closeFailure=IllegalStateException("retirement pending")
        val first=runCatching {
            retireAssistedBeforeAdmission({ order+="close";throw closeFailure },
                { order+="idle";retained==null })
        }
        assertSame(closeFailure,first.exceptionOrNull())
        assertSame(stale,retained)
        assertEquals(listOf("close"),order)
        order.clear()
        val admitted=retireAssistedBeforeAdmission({order+="close";retained=null},
            {order+="idle";retained==null})
        assertTrue(admitted)
        assertEquals(listOf("close","idle"),order)
    }
}
