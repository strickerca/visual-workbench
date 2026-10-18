package com.visualworkbench.android.documents

import android.graphics.*
import com.visualworkbench.shared.*
import java.util.IdentityHashMap

internal data class PreparedDocumentTile(val source:DocumentDisplayTile,val bitmap:Bitmap)
internal class DocumentTileCache {
    private val images=IdentityHashMap<DocumentDisplayTile,Bitmap>()
    @Synchronized fun prepare(tiles:List<DocumentDisplayTile>):List<PreparedDocumentTile> {
        require(tiles.size<=32);val retained=IdentityHashMap<DocumentDisplayTile,Bitmap>()
        val result=tiles.map{tile ->
            val image=images[tile] ?: run {
                require(tile.width<=512u && tile.height<=512u && tile.rgba.size==tile.width.toInt()*tile.height.toInt()*4)
                val pixels=IntArray(tile.width.toInt()*tile.height.toInt())
                for(i in pixels.indices){val b=i*4;pixels[i]=((tile.rgba[b+3].toInt() and 255) shl 24) or ((tile.rgba[b].toInt() and 255) shl 16) or ((tile.rgba[b+1].toInt() and 255) shl 8) or (tile.rgba[b+2].toInt() and 255)}
                Bitmap.createBitmap(pixels,tile.width.toInt(),tile.height.toInt(),Bitmap.Config.ARGB_8888)
            }
            retained[tile]=image;PreparedDocumentTile(tile,image)
        }
        // Render-session frames may still reference retired bitmaps. GC owns
        // release after those immutable frames retire; never recycle in flight.
        images.clear();images.putAll(retained);return result
    }
}
internal fun drawDocumentTiles(canvas:Canvas,tiles:List<PreparedDocumentTile>) {
    val paint=Paint(Paint.FILTER_BITMAP_FLAG)
    for(tile in tiles){canvas.save();canvas.translate(tile.source.origin.x.toFloat(),tile.source.origin.y.toFloat());canvas.scale((1.0/tile.source.scale).toFloat(),(1.0/tile.source.scale).toFloat());canvas.drawBitmap(tile.bitmap,0f,0f,paint);canvas.restore()}
}
