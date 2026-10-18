package com.visualworkbench.desktop
import org.junit.Assert.*
import org.junit.Test

class GiantCanvasBoundaryTest {
    @Test fun nestedSceneRoutesEachFocusedShortcutOnceWithCurrentModalPolicy(){
        val route=GiantShortcutRoute<Command>();val received=mutableListOf<Command>()
        var focused=true;var modal=false
        val windowHandler:(Command)->Boolean={command->
            if(shortcutAllowed(focused,modal,command==Command.Undo,false)){received.add(command);true}else false
        }
        route.update(windowHandler)
        for(command in listOf(Command.Delete,Command.ClearSelection,Command.Undo,Command.CycleMarker))assertTrue(route.dispatch(command))
        assertEquals(listOf(Command.Delete,Command.ClearSelection,Command.Undo,Command.CycleMarker),received)
        modal=true;assertFalse(route.dispatch(Command.Undo));assertFalse(route.dispatch(Command.Delete));assertEquals(4,received.size)
        modal=false;focused=false;assertFalse(route.dispatch(Command.Delete))
        // A current replacement handler takes effect in the owned scene without
        // an AWT/global re-dispatch to the outer scene.
        route.update{received.add(Command.Fit);true};assertTrue(route.dispatch(Command.Delete));assertEquals(Command.Fit,received.last())
        route.seal();route.update(windowHandler);assertFalse(route.dispatch(Command.Delete));assertEquals(5,received.size)
    }

    @Test fun physicalSurfaceAdmissionAccountsForMonitorScale(){
        assertTrue(giantSurfaceAdmitted(2048,2048,2.0,2.0))
        assertFalse(giantSurfaceAdmitted(2049,2048,2.0,2.0))
        assertFalse(giantSurfaceAdmitted(8193,1,1.0,1.0))
        assertFalse(giantSurfaceAdmitted(1,1,Double.NaN,1.0))
    }
    @Test fun giantExportNeverSilentlyConvertsOrResizes(){
        val plain=ExportRequest(1,"project","document",2uL,"a".repeat(64),ExportSettings(assumeUntaggedSrgb=true),null,20000u,10000u,emptySet(),null)
        admitGiantExport(plain)
        for(settings in listOf(plain.settings.copy(encoding=ExportEncoding.Jpeg),plain.settings.copy(matte=ExportMatte.White),plain.settings.copy(convertToSrgb=true),plain.settings.copy(allowDepthReduction=true),plain.settings.copy(assumeUntaggedSrgb=false))){
            try{admitGiantExport(plain.copy(settings=settings));fail("conversion must refuse")}catch(_:HandoffFailure){}
        }
    }
}
