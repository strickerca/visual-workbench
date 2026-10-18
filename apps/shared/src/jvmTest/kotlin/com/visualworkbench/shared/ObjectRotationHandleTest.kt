@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class ObjectRotationHandleTest {
    private fun document(locked:Boolean=false):DocumentSnapshot {
        val bounds=Rect(10.0,20.0,40.0,20.0)
        val item=RenderItem("object","layer",1.0,"normal",bounds,Contours(longArrayOf(),longArrayOf(),uintArrayOf()),
            Transform(),ObjectStyle(0xffffffffu,1.0),Shape.Rectangle(bounds),locked)
        return DocumentSnapshot("doc","Test",100u,100u,8u,emptyList(),RenderList(
            ProjectInfo("project","Test","device",1u,false,false,1u,"a".repeat(64),listOf("doc")),listOf(item)))
    }
    private val camera=Camera(Point(50.0,50.0),2.0,0.0,200.0,200.0)
    @Test fun offsetRemainsPhysicalAndHitUsesTheActualForwardPaneMatrix(){
        val handle=checkNotNull(objectRotationHandle(document(),setOf("object"),camera))
        assertEquals(Point(30.0,30.0),handle.pivot);assertEquals(Point(30.0,6.0),handle.point)
        val physical=Transform(0.0,3.0,-3.0,0.0,100.0,200.0)
        assertTrue(handle.hit(physical,Point(82.0,290.0),12.0))
        assertFalse(handle.hit(physical,Point(95.0,290.0),12.0))
    }
    @Test fun lockedMissingAndEmptySelectionsHaveNoMutationHandle(){
        assertNull(objectRotationHandle(document(true),setOf("object"),camera))
        assertNull(objectRotationHandle(document(),setOf("absent"),camera))
        assertNull(objectRotationHandle(document(),emptySet(),camera))
    }
    @Test fun invalidDisplayGeometryNeverAcquiresAHit(){
        assertNull(objectRotationHandle(document(),setOf("object"),camera.copy(scale=Double.NaN)))
        val handle=checkNotNull(objectRotationHandle(document(),setOf("object"),camera))
        assertFalse(handle.hit(Transform(a=Double.NaN),Point(0.0,0.0),12.0))
        assertFalse(handle.hit(Transform(),Point(Double.POSITIVE_INFINITY,0.0),12.0))
        assertFalse(handle.hit(Transform(),handle.point,Double.NaN))
    }
}
