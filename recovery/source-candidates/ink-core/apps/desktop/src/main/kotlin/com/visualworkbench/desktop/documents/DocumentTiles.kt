package com.visualworkbench.desktop.documents

import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import com.visualworkbench.shared.*
import java.awt.image.BufferedImage
import java.util.IdentityHashMap

internal data class PreparedDocumentTile(val source:DocumentDisplayTile,val image:ImageBitmap)
internal class DocumentTileCache {
    private val images=IdentityHashMap<DocumentDisplayTile,ImageBitmap>()
    @Synchronized fun prepare(tiles:List<DocumentDisplayTile>):List<PreparedDocumentTile> {
        require(tiles.size<=32);val retained=IdentityHashMap<DocumentDisplayTile,ImageBitmap>()
        val result=tiles.map{tile ->
            val image=images[tile] ?: run {
                require(tile.width<=512u && tile.height<=512u && tile.rgba.size==tile.width.toInt()*tile.height.toInt()*4)
                val source=BufferedImage(tile.width.toInt(),tile.height.toInt(),BufferedImage.TYPE_INT_ARGB)
                for(y in 0 until source.height)for(x in 0 until source.width){val i=(y*source.width+x)*4;source.setRGB(x,y,((tile.rgba[i+3].toInt() and 255) shl 24) or ((tile.rgba[i].toInt() and 255) shl 16) or ((tile.rgba[i+1].toInt() and 255) shl 8) or (tile.rgba[i+2].toInt() and 255))}
                source.toComposeImageBitmap()
            }
            retained[tile]=image;PreparedDocumentTile(tile,image)
        }
        images.clear();images.putAll(retained);return result
    }
}
internal fun DrawScope.drawDocumentTiles(tiles:List<PreparedDocumentTile>) {
    for(tile in tiles)withTransform({translate(tile.source.origin.x.toFloat(),tile.source.origin.y.toFloat());scale((1.0/tile.source.scale).toFloat(),(1.0/tile.source.scale).toFloat(),androidx.compose.ui.geometry.Offset.Zero)}){drawImage(tile.image,filterQuality=FilterQuality.Low)}
}
