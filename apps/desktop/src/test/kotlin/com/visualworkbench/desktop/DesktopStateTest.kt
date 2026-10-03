package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path

class DesktopStateTest {
    @Test fun remoteEditsAndViewUpdatesNeverChangeAnUnfollowedLocalCamera(){
        val camera=Camera(Point(123456.125,-876.25),2.5,.375,1280.0,720.0)
        var view=ViewState(camera)
        repeat(100){n->view=view.receivePeerEdit().receivePeerView(Camera(Point(n.toDouble(),n*2.0),1.0+n*.1,0.0,300.0,600.0))}
        assertEquals(camera.center.x.toBits(),view.camera.center.x.toBits());assertEquals(camera.center.y.toBits(),view.camera.center.y.toBits())
        assertEquals(camera.scale.toBits(),view.camera.scale.toBits());assertEquals(camera.rotation.toBits(),view.camera.rotation.toBits())
        assertFalse(view.followPeer);assertFalse(view.showPeerOutline)
        val matched=view.matchPeer();assertEquals(Point(99.0,198.0),matched.camera.center);assertEquals(1280.0,matched.camera.viewportWidth,0.0)
        val followed=view.copy(followPeer=true).receivePeerView(Camera(Point(7.0,8.0),3.0,.2,10.0,20.0));assertEquals(Point(7.0,8.0),followed.camera.center);assertEquals(720.0,followed.camera.viewportHeight,0.0)
    }
    @Test fun allWindowModesPersistTheirWindowedBounds(){
        val directory=Files.createTempDirectory("vw-desktop-state-").toFile()
        try{val file=directory.toPath().resolve("settings.properties");for(mode in WindowMode.entries){val value=SavedWindow(mode,1234f,789f,-120f,54f);DesktopPreferences(file).saveWindow(value);assertEquals(value,DesktopPreferences(file).window())}}
        finally{check(directory.name.startsWith("vw-desktop-state-"));check(directory.canonicalFile.parentFile==directory.absoluteFile.parentFile.canonicalFile);check(directory.deleteRecursively())}
    }
    @Test fun cloudDocumentsPreferLocalProjectStorage(){
        val local=Path.of("C:/fixture/Local");val cloud=Path.of("C:/fixture/Cloud")
        val fallback=ProjectLocations.choose(cloud.resolve("Documents"),local,listOf(cloud));assertTrue(fallback.usesLocalAppData);assertEquals(local.resolve("Visual Workbench/Projects"),fallback.path)
        assertTrue(ProjectLocations.choose(Path.of("C:/fixture/OneDrive - example/Documents"),local,emptyList()).usesLocalAppData)
        val normal=ProjectLocations.choose(Path.of("C:/fixture/Documents"),local,listOf(cloud));assertFalse(normal.usesLocalAppData)
    }
    @Test fun nudgeAndResizeOperateInDocumentCoordinates(){
        val original=Transform(a=2.0,d=3.0,e=100.0,f=50.0)
        val one=translated(original,1.0,-1.0);val ten=translated(original,10.0,-10.0)
        assertEquals(101.0,one.e,0.0);assertEquals(49.0,one.f,0.0);assertEquals(110.0,ten.e,0.0);assertEquals(40.0,ten.f,0.0)
        val resized=scaledAbout(original,Point(100.0,50.0),2.0,.5);assertEquals(4.0,resized.a,0.0);assertEquals(1.5,resized.d,0.0);assertEquals(100.0,resized.e,0.0);assertEquals(50.0,resized.f,0.0)
    }
}
