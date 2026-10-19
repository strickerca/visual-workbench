package com.visualworkbench.android.assistedselection

import com.visualworkbench.shared.*
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
}
