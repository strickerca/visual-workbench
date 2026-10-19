@file:OptIn(ExperimentalUnsignedTypes::class)

package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test

class ComposedCanvasPolicyTest {
    private val revision=ProjectInfo("project","image","device",1uL,false,false,2uL,"a".repeat(64),listOf("document"))
    private fun item(id:String,shape:Shape)=RenderItem(id,"layer",1.0,"normal",Rect(0.0,0.0,4.0,4.0),
        Contours(longArrayOf(),longArrayOf(),uintArrayOf()),Transform(),ObjectStyle(0xffffffffu,1.0),shape,false)
    private val document=DocumentSnapshot("document","image",16u,16u,8u,emptyList(),RenderList(revision,listOf(
        item("art",Shape.Rectangle(Rect(0.0,0.0,4.0,4.0))),
        item("result",Shape.Result("b".repeat(64))),
        item("guide",Shape.Guide(Rect(0.0,0.0,4.0,4.0))),
        item("tone",Shape.Adjustment))),hasToneAdjustments=true)

    @Test fun onlyCanonicalContentActuallyBakedIsSuppressed() {
        val baked=bakedCanonicalIds(document,true)
        assertEquals(setOf("art","result"),baked)
        assertTrue(bakedCanonicalIds(document,false).isEmpty())
        assertEquals(setOf("guide"),transientHiddenIds(baked+"guide",baked))
    }

    @Test fun movedSavedObjectStaysBakedWhileNewTransientRemainsDrawable() {
        val baked=bakedCanonicalIds(document,true)
        val movedSaved=item("art",Shape.Rectangle(Rect(8.0,0.0,4.0,4.0)))
        val newStroke=item("new",Shape.Stroke("pen"))
        val renderProvisional=listOf(movedSaved,newStroke).filterNot { it.objectId in baked }
        assertEquals(listOf("new"),renderProvisional.map { it.objectId })
        assertEquals(setOf("art","result"),baked) // Original pixels remain until commit.
    }

    @Test fun orderedChangeStillReloadsPixelsWhenUndoRemovesTheLastTone() {
        val obligation=ComposedReloadObligation()
        assertFalse(obligation.needsReload(false))
        obligation.revokeComposedPixels() // Observe hides the stale bitmap before the new document arrives.
        assertTrue(obligation.pending)
        assertTrue(obligation.needsReload(false)) // The refreshed document has no tone.
        obligation.settle()
        assertFalse(obligation.needsReload(false))
        assertTrue(obligation.needsReload(true))
    }
}
