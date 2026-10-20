@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.material.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.ImageComposeScene
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import org.junit.Assert.*
import org.junit.Test

class DesktopStreamPanelLayoutTest {
    @Test fun realComposeLayoutKeepsContactOriginAndViewportWhenFrozenToolsAndIssuesArrive(){
        val frozen=mutableStateOf(false)
        var origin:Offset?=null
        var size:IntSize?=null
        val scene=ImageComposeScene(800,600,Density(1f)){
            StreamPanelLayout(Modifier.fillMaxSize(),
                header={Text(if(frozen.value)"FROZEN · exact source · original pending" else "LIVE · source")},
                issue={if(frozen.value)Text("A newly arrived multi-line issue must not move this contact. ".repeat(6))},
                tools={if(frozen.value)Row{repeat(8){Text("Tool $it ")}}},
                canvas={Box(Modifier.fillMaxSize().onGloballyPositioned{origin=it.positionInRoot();size=it.size})})
        }
        try{
            scene.render(0).close()
            val liveOrigin=checkNotNull(origin);val liveSize=checkNotNull(size)
            assertEquals(Offset(0f,184f),liveOrigin)
            assertEquals(IntSize(800,416),liveSize)
            val physicalDown=Offset(123f,245f)-liveOrigin
            frozen.value=true;Snapshot.sendApplyNotifications()
            scene.render(16_000_000).close()
            assertEquals(liveOrigin,origin)
            assertEquals(liveSize,size)
            assertEquals(physicalDown,Offset(123f,245f)-checkNotNull(origin))
        }finally{scene.close()}
    }
}
