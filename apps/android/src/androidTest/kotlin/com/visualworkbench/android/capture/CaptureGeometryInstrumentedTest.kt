package com.visualworkbench.android.capture

import android.graphics.Rect
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Synthetic geometry only: never enables accessibility or takes a screenshot. */
@RunWith(AndroidJUnit4::class)
class CaptureGeometryInstrumentedTest {
    @Test fun ownAppAndNotificationShadeAreNotTargets(){
        for(name in listOf("test.owner","com.android.systemui"))try{
            CaptureAdmission.target(Target(1,name,Rect(0,0,100,100),0),"test.owner");fail("own surface admitted")
        }catch(error:CaptureRefusal){assertEquals(CaptureRefusal.Reason.OwnWindow,error.reason)}
    }
    @Test fun sameWindowIdDoesNotPermitMovedOrRotatedGeometry(){
        val original=Target(1,"test.source",Rect(0,0,100,100),0)
        for(changed in listOf(original.copy(bounds=Rect(1,0,101,100)),original.copy(rotation=1),original.copy(windowId=2)))
            try{original.same(changed);fail("stale geometry admitted")}catch(error:CaptureRefusal){assertEquals(CaptureRefusal.Reason.Stale,error.reason)}
    }
    @Test fun absentWindowAndEmptyExtentFailClosed(){
        for(value in listOf(Target(-1,"test.source",Rect(0,0,1,1),0),Target(1,"test.source",Rect(0,0,0,1),0)))
            try{CaptureAdmission.target(value,"test.owner");fail("missing target admitted")}catch(error:CaptureRefusal){assertEquals(CaptureRefusal.Reason.Unavailable,error.reason)}
    }
}
