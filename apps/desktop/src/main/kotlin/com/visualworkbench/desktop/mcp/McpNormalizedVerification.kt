package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.McpInboxPixels
import com.visualworkbench.shared.McpInboxRegion
import com.visualworkbench.shared.McpInboxSide
import com.visualworkbench.shared.Rect
import com.visualworkbench.shared.VerificationImageCoordinates
import kotlin.math.ceil
import kotlin.math.floor

/** Reproject checked retained-PNG pixels onto a common D-space sample grid.
 * This is an observation for comparison/registration, never capture or verdict
 * authority. A source pixel is used only when its entire square lies inside
 * the continuous content bounds; rounded-up edge and 64px padding are excluded.
 * Reads use the existing receipt owner in <=512px tiles, with at most 16 tiles
 * and 262,144 output samples. No source PNG or full-frame copy is requested. */
internal suspend fun normalizedVerificationPixels(
    receiptId:String, receiptHash:String, side:McpInboxSide,
    geometry:VerificationImageCoordinates, document:Rect, width:Int, height:Int,
    read:suspend(McpInboxRegion)->McpInboxPixels, cancelled:()->Boolean={false},
):McpInboxPixels? {
    if(width !in 1..512 || height !in 1..512 || !verificationInsideCrop(document,geometry) ||
        geometry.documentToImage.size!=6 || geometry.contentBounds.size!=4 || cancelled() ||
        geometry.imageWidth !in 1u..8192u || geometry.imageHeight !in 1u..8192u)return null
    val m=geometry.documentToImage;val b=geometry.contentBounds
    if((m+b).any{!it.isFinite()} || b[2]<=0 || b[3]<=0)return null
    val contentRight=b[0]+b[2];val contentBottom=b[1]+b[3]
    if(!contentRight.isFinite()||!contentBottom.isFinite())return null
    val count=width*height;val sx=IntArray(count);val sy=IntArray(count)
    var minX=Int.MAX_VALUE;var minY=Int.MAX_VALUE;var maxX=-1;var maxY=-1
    for(y in 0 until height){
        if(cancelled())return null
        val dy=document.y+(y+0.5)*document.height/height
        for(x in 0 until width){
            val dx=document.x+(x+0.5)*document.width/width
            val qx=m[0]*dx+m[2]*dy+m[4];val qy=m[1]*dx+m[3]*dy+m[5]
            if(!qx.isFinite()||!qy.isFinite()||qx<0||qy<0||
                qx>=geometry.imageWidth.toDouble()||qy>=geometry.imageHeight.toDouble())return null
            val px=floor(qx);val py=floor(qy)
            // Source pixel support, not merely the mapped sample point, must
            // be inside the real continuous content envelope.
            if(px<b[0]||py<b[1]||px+1>contentRight||py+1>contentBottom)return null
            val index=y*width+x;val ix=px.toInt();val iy=py.toInt()
            sx[index]=ix;sy[index]=iy
            minX=minOf(minX,ix);minY=minOf(minY,iy);maxX=maxOf(maxX,ix);maxY=maxOf(maxY,iy)
        }
    }
    val firstTileX=minX/512;val lastTileX=maxX/512
    val firstTileY=minY/512;val lastTileY=maxY/512
    if((lastTileX-firstTileX+1).toLong()*(lastTileY-firstTileY+1)>16L)return null
    val rgba=ByteArray(count*4)
    for(tileY in firstTileY..lastTileY)for(tileX in firstTileX..lastTileX){
        if(cancelled())return null
        val left=tileX*512;val top=tileY*512
        val tileWidth=minOf(512,geometry.imageWidth.toInt()-left)
        val tileHeight=minOf(512,geometry.imageHeight.toInt()-top)
        if(tileWidth<=0||tileHeight<=0)return null
        val region=McpInboxRegion(left.toUInt(),top.toUInt(),tileWidth.toUInt(),tileHeight.toUInt())
        val tile=read(region)
        if(tile.receiptId!=receiptId||tile.receiptBlake3!=receiptHash||tile.side!=side||tile.region!=region||
            tile.canvasWidth!=geometry.imageWidth||tile.canvasHeight!=geometry.imageHeight||
            tile.rgbaSrgb.size!=tileWidth*tileHeight*4)return null
        for(i in 0 until count){
            if((i and 1023)==0&&cancelled())return null
            val x=sx[i]-left;val y=sy[i]-top
            if(x in 0 until tileWidth&&y in 0 until tileHeight)
                tile.rgbaSrgb.copyInto(rgba,i*4,(y*tileWidth+x)*4,(y*tileWidth+x)*4+4)
        }
    }
    return McpInboxPixels(receiptId,receiptHash,side,McpInboxRegion(0u,0u,width.toUInt(),height.toUInt()),
        width.toUInt(),height.toUInt(),rgba)
}

internal fun verificationInsideCrop(region:Rect,geometry:VerificationImageCoordinates):Boolean {
    val c=geometry.cropDocument
    return c.size==4 && listOf(region.x,region.y,region.width,region.height).all{it.isFinite()} &&
        c.all{it.isFinite()} && region.width>0 && region.height>0 && c[2]>0 && c[3]>0 &&
        region.x>=c[0] && region.y>=c[1] && region.x+region.width<=c[0]+c[2] &&
        region.y+region.height<=c[1]+c[3]
}

/** Whole-pixel common D region for conservative image registration. Both
 * crops must share this region. Alignment outside it remains unresolved. */
internal fun verificationRegistrationRegion(before:VerificationImageCoordinates,
    after:VerificationImageCoordinates,focus:Rect):Rect? {
    val a=before.cropDocument;val b=after.cropDocument
    if(a.size!=4||b.size!=4||focus.width<=0||focus.height<=0)return null
    val left=ceil(maxOf(a[0],b[0]));val top=ceil(maxOf(a[1],b[1]))
    val right=floor(minOf(a[0]+a[2],b[0]+b[2]))
    val bottom=floor(minOf(a[1]+a[3],b[1]+b[3]))
    if(!listOf(left,top,right,bottom).all{it.isFinite()}||right-left<96||bottom-top<96)return null
    val width=minOf(512.0,right-left);val height=minOf(512.0,bottom-top)
    val x=floor(focus.x+focus.width/2-width/2).coerceIn(left,right-width)
    val y=floor(focus.y+focus.height/2-height/2).coerceIn(top,bottom-height)
    return Rect(x,y,width,height)
}

internal fun verificationChangeRegions(markers:List<Rect>,tile:Rect):List<Rect> = markers.mapNotNull{box->
    val left=maxOf(box.x,tile.x);val top=maxOf(box.y,tile.y)
    val right=minOf(box.x+box.width,tile.x+tile.width);val bottom=minOf(box.y+box.height,tile.y+tile.height)
    if(right>left&&bottom>top)Rect(left-tile.x,top-tile.y,right-left,bottom-top)else null
}
