package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

/** Synthetic geometry/pixel goldens, not an OS recapture or device result. */
class McpNormalizedVerificationTest {
    private val key=VerificationPackageKey("019a3aaa-0123-789a-8abc-000000000001","generic","a".repeat(64))
    private val binding=WorkflowBinding("019a3aaa-0123-789a-8abc-000000000002","019a3aaa-0123-789a-8abc-000000000003",1uL,"b".repeat(64))
    private fun image(sourceWidth:Int,sourceHeight:Int,width:Int,height:Int,content:List<Double>,crop:List<Double>,matrix:List<Double>):VerificationImageCoordinates =
        VerificationImageCoordinates(key,binding,"c".repeat(64),sourceWidth.toUInt(),sourceHeight.toUInt(),width.toUInt(),height.toUInt(),
            width.toUInt(),height.toUInt(),ceilContent(content[2]),ceilContent(content[3]),content,crop,matrix,
            listOf(1.0,0.0,0.0,1.0,0.0,0.0),matrix!=listOf(1.0,0.0,0.0,1.0,0.0,0.0))
    private fun ceilContent(value:Double)=kotlin.math.ceil(value).toUInt()
    private fun pixels(width:Int,height:Int,pattern:(Int,Int)->Int):ByteArray = ByteArray(width*height*4).also{bytes->
        for(y in 0 until height)for(x in 0 until width){val n=(y*width+x)*4;val v=pattern(x,y)
            bytes[n]=v.toByte();bytes[n+1]=(v ushr 8).toByte();bytes[n+2]=(v ushr 16).toByte();bytes[n+3]=255.toByte()}
    }
    private fun reader(bytes:ByteArray,width:Int,height:Int,side:McpInboxSide):suspend(McpInboxRegion)->McpInboxPixels = {region->
        val tile=ByteArray(region.width.toInt()*region.height.toInt()*4)
        for(y in 0 until region.height.toInt())for(x in 0 until region.width.toInt())
            bytes.copyInto(tile,(y*region.width.toInt()+x)*4,((region.y.toInt()+y)*width+region.x.toInt()+x)*4,
                ((region.y.toInt()+y)*width+region.x.toInt()+x)*4+4)
        McpInboxPixels("receipt","hash",side,region,width.toUInt(),height.toUInt(),tile)
    }
    @Test fun quarterTurnAndUnequalResizeNormalizeToTheSameDocumentPixels()=runBlocking {
        val before=image(4,3,64,64,listOf(0.0,0.0,4.0,3.0),listOf(0.0,0.0,4.0,3.0),listOf(1.0,0.0,0.0,1.0,0.0,0.0))
        val turn=image(4,3,64,64,listOf(0.0,0.0,3.0,4.0),listOf(0.0,0.0,4.0,3.0),listOf(0.0,1.0,-1.0,0.0,3.0,0.0))
        val resized=image(4,3,64,64,listOf(0.0,0.0,8.0,6.0),listOf(0.0,0.0,4.0,3.0),listOf(2.0,0.0,0.0,2.0,0.0,0.0))
        fun color(x:Int,y:Int)=x*113+y*271+7
        val a=pixels(64,64){x,y->if(x<4&&y<3)color(x,y)else 0}
        val b=pixels(64,64){x,y->if(x<3&&y<4)color(y,2-x)else 0}
        val c=pixels(64,64){x,y->if(x<8&&y<6)color(x/2,y/2)else 0}
        val region=Rect(0.0,0.0,4.0,3.0)
        val first=normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,before,region,4,3,reader(a,64,64,McpInboxSide.Before))
        val rotated=normalizedVerificationPixels("receipt","hash",McpInboxSide.After,turn,region,4,3,reader(b,64,64,McpInboxSide.After))
        val scaled=normalizedVerificationPixels("receipt","hash",McpInboxSide.After,resized,region,4,3,reader(c,64,64,McpInboxSide.After))
        assertNotNull(first);assertArrayEquals(first!!.rgbaSrgb,rotated!!.rgbaSrgb);assertArrayEquals(first.rgbaSrgb,scaled!!.rgbaSrgb)
    }
    @Test fun cropAndFractionalPaddingRefuseUnsupportedSamples()=runBlocking {
        val frame=image(5,3,64,64,listOf(0.0,0.0,2.5,1.5),listOf(0.0,0.0,5.0,3.0),listOf(0.5,0.0,0.0,0.5,0.0,0.0))
        val bytes=pixels(64,64){_,_->23}
        assertNull(normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,frame,Rect(4.0,0.0,1.0,1.0),1,1,reader(bytes,64,64,McpInboxSide.Before)))
        val cropped=frame.copy(cropDocument=listOf(1.0,0.0,4.0,3.0))
        assertNull(normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,cropped,Rect(0.0,0.0,2.0,1.0),2,1,reader(bytes,64,64,McpInboxSide.Before)))
        assertNotNull(normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,frame,Rect(2.0,0.0,1.0,1.0),1,1,reader(bytes,64,64,McpInboxSide.Before)))
    }
    @Test fun normalizedRotatedGridRegistersOneUniqueDocumentTranslation()=runBlocking {
        val size=128
        val before=image(size,size,size,size,listOf(0.0,0.0,128.0,128.0),listOf(0.0,0.0,128.0,128.0),listOf(1.0,0.0,0.0,1.0,0.0,0.0))
        val after=image(size,size,size,size,listOf(0.0,0.0,128.0,128.0),listOf(0.0,0.0,128.0,128.0),listOf(0.0,1.0,-1.0,0.0,128.0,0.0))
        fun color(x:Int,y:Int)=if(x in 0 until size&&y in 0 until size) ((x*73+y*151) xor (x*y*19)) and 0xffffff else 0
        val a=pixels(size,size){x,y->color(x,y)}
        val b=pixels(size,size){x,y->color(y-5,127-x-3)}
        val region=Rect(0.0,0.0,128.0,128.0)
        val p=normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,before,region,size,size,reader(a,size,size,McpInboxSide.Before))!!
        val q=normalizedVerificationPixels("receipt","hash",McpInboxSide.After,after,region,size,size,reader(b,size,size,McpInboxSide.After))!!
        val match=registerMcpImages(p,q,emptyList(),emptyList())
        assertTrue(match.toString(),match is McpImageRegistrationResult.Aligned)
        match as McpImageRegistrationResult.Aligned
        assertEquals(5,match.dx);assertEquals(3,match.dy)
    }
    @Test fun tileProvenanceAndBoundedWorkRefuseBeforeUsingWrongPixels()=runBlocking {
        val plain=image(64,64,64,64,listOf(0.0,0.0,64.0,64.0),listOf(0.0,0.0,64.0,64.0),listOf(1.0,0.0,0.0,1.0,0.0,0.0))
        val bytes=pixels(64,64){_,_->21}
        val wrong=reader(bytes,64,64,McpInboxSide.After)
        assertNull(normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,plain,Rect(1.0,1.0,4.0,4.0),4,4,wrong))
        val zoom=image(512,512,8192,8192,listOf(0.0,0.0,8192.0,8192.0),listOf(0.0,0.0,512.0,512.0),
            listOf(16.0,0.0,0.0,16.0,0.0,0.0))
        var reads=0
        assertNull(normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,zoom,Rect(0.0,0.0,512.0,512.0),512,512,
            {reads++;error("Tile budget should refuse before read") }))
        assertEquals(0,reads)
    }
    @Test fun samplesAcrossTwoPhysicalTilesKeepTheirDocumentOrder()=runBlocking {
        val shifted=image(200,1,1024,64,listOf(0.0,0.0,1024.0,64.0),listOf(0.0,0.0,200.0,1.0),
            listOf(1.0,0.0,0.0,1.0,500.0,0.0))
        val bytes=pixels(1024,64){x,_->x}
        val requested=mutableListOf<McpInboxRegion>()
        val read=reader(bytes,1024,64,McpInboxSide.Before)
        val result=normalizedVerificationPixels("receipt","hash",McpInboxSide.Before,shifted,Rect(0.0,0.0,200.0,1.0),200,1,
            {tile->requested+=tile;read(tile)})!!
        assertEquals(2,requested.size)
        assertEquals(500,((result.rgbaSrgb[1].toInt() and 255) shl 8) or (result.rgbaSrgb[0].toInt() and 255))
        val last=(199*4)
        assertEquals(699,((result.rgbaSrgb[last+1].toInt() and 255) shl 8) or (result.rgbaSrgb[last].toInt() and 255))
    }
}
