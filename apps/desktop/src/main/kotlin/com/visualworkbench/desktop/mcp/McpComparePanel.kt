package com.visualworkbench.desktop.mcp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.jetbrains.skia.Image
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import javax.imageio.ImageIO

private class InboxTile(val native:Image,val bitmap:ImageBitmap):AutoCloseable{override fun close(){native.close()}}
private fun inboxTile(value:McpInboxPixels):InboxTile {
    val width=value.region.width.toInt();val height=value.region.height.toInt()
    require(width in 1..512&&height in 1..512&&value.rgbaSrgb.size==width*height*4)
    val raster=BufferedImage(width,height,BufferedImage.TYPE_INT_ARGB)
    val pixels=IntArray(width*height)
    for(i in pixels.indices){val at=i*4;pixels[i]=((value.rgbaSrgb[at+3].toInt()and 255) shl 24) or ((value.rgbaSrgb[at].toInt()and 255) shl 16) or ((value.rgbaSrgb[at+1].toInt()and 255) shl 8) or (value.rgbaSrgb[at+2].toInt()and 255)}
    raster.setRGB(0,0,width,height,pixels,0,width)
    val encoded=ByteArrayOutputStream().use{check(ImageIO.write(raster,"png",it));it.toByteArray()}
    require(encoded.size<=2*1024*1024)
    val image=Image.makeFromEncoded(encoded)
    return try{InboxTile(image,image.toComposeImageBitmap())}catch(error:Throwable){image.close();throw error}
}

/** Literal untrusted content, exact immutable receipt, bounded native display
 * conversions. No callback accepts a Result or mutates the editor document. */
@Composable internal fun McpComparePanel(owner:DesktopMcpCoordinator,receipt:McpInboxReceipt,onClose:()->Unit,onExport:(McpInboxReceipt,McpInboxSide)->Unit) {
    var xText by remember(receipt.receiptId){mutableStateOf("0")};var yText by remember(receipt.receiptId){mutableStateOf("0")}
    var assumeSrgb by remember(receipt.receiptId){mutableStateOf(false)};var allowDepth by remember(receipt.receiptId){mutableStateOf(false)}
    var before by remember(receipt.receiptId){mutableStateOf<InboxTile?>(null)};var after by remember(receipt.receiptId){mutableStateOf<InboxTile?>(null)}
    var issue by remember(receipt.receiptId){mutableStateOf<String?>(null)};var loading by remember{mutableStateOf(false)}
    val x=xText.toUIntOrNull();val y=yText.toUIntOrNull()
    LaunchedEffect(receipt.receiptId,receipt.receiptBlake3,x,y,assumeSrgb,allowDepth) {
        before=null;after=null;issue=null
        if(x==null||y==null){issue="Enter nonnegative pixel coordinates.";return@LaunchedEffect}
        val width=minOf(receipt.before.width,receipt.after?.width?:receipt.before.width)
        val height=minOf(receipt.before.height,receipt.after?.height?:receipt.before.height)
        if(x>=width||y>=height){issue="The region is outside the shared Before/After extent.";return@LaunchedEffect}
        val region=McpInboxRegion(x,y,minOf(512u,width-x),minOf(512u,height-y))
        var ownedBefore:InboxTile?=null;var ownedAfter:InboxTile?=null
        loading=true
        try {
            val beforePixels=owner.pixels(receipt,McpInboxSide.Before,region,assumeSrgb,allowDepth)
            // No dispatcher handoff after acquiring a native image. These <=1MiB
            // conversions run in this effect; native full-image decode is off-UI.
            ownedBefore=inboxTile(beforePixels)
            if(receipt.after!=null)ownedAfter=inboxTile(owner.pixels(receipt,McpInboxSide.After,region,assumeSrgb,allowDepth))
            currentCoroutineContext().ensureActive();before=ownedBefore;ownedBefore=null;after=ownedAfter;ownedAfter=null
        }catch(error:CancellationException){throw error}
        catch(_:Exception){issue="Display refused. Review color/depth permissions, the region, or receipt integrity."}
        finally{ownedBefore?.close();ownedAfter?.close();loading=false}
    }
    DisposableEffect(before){val tile=before;onDispose{tile?.close()}}
    DisposableEffect(after){val tile=after;onDispose{tile?.close()}}
    AlertDialog(onDismissRequest=onClose,title={Text("Compare agent return")},text={Column(Modifier.widthIn(max=1100.dp).heightIn(max=750.dp).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(10.dp)){
        Text("Untrusted external content. No exterior-pixel proof. Arrival never changes the document.",color=MaterialTheme.colors.secondary)
        Text("Package ${receipt.packageId} | revision ${receipt.binding.hostSeq}",style=MaterialTheme.typography.caption)
        Text("Manifest ${receipt.manifestSha256}",style=MaterialTheme.typography.caption)
        receipt.text?.let{Text(it)};if(receipt.note.isNotEmpty())Text(receipt.note)
        Row(horizontalArrangement=Arrangement.spacedBy(8.dp)){
            OutlinedTextField(xText,{xText=it.take(10)},Modifier.width(150.dp),label={Text("Region X | pixels")})
            OutlinedTextField(yText,{yText=it.take(10)},Modifier.width(150.dp),label={Text("Region Y | pixels")})
        }
        Row{Checkbox(assumeSrgb,{assumeSrgb=it});Text("Treat untagged display colors as sRGB")}
        if(receipt.before.bitDepth.toInt()>8||(receipt.after?.bitDepth?.toInt()?:8)>8)Row{Checkbox(allowDepth,{allowDepth=it});Text("Allow 16-to-8-bit display conversion; keep originals intact")}
        if(loading)LinearProgressIndicator(Modifier.fillMaxWidth())
        issue?.let{Text(it,color=MaterialTheme.colors.error)}
        val density=LocalDensity.current.density
        Row(Modifier.horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(12.dp)){
            before?.let{tile->Column{Text("Before | immutable package image");Image(tile.bitmap,"Before",Modifier.size((tile.bitmap.width/density).dp,(tile.bitmap.height/density).dp),contentScale=ContentScale.None)}}
            after?.let{tile->Column{Text("After | untrusted agent PNG");Image(tile.bitmap,"After",Modifier.size((tile.bitmap.width/density).dp,(tile.bitmap.height/density).dp),contentScale=ContentScale.None)}}
        }
        Text("Full-resolution region up to 512 x 512 pixels. Different image sizes remain explicit; no stretch or hidden resampling.",style=MaterialTheme.typography.caption)
        Row{TextButton(onClick={onExport(receipt,McpInboxSide.Before)}){Text("Save exact Before PNG")};if(receipt.after!=null)TextButton(onClick={onExport(receipt,McpInboxSide.After)}){Text("Save exact After PNG")}}
        Text("Canonical Result acceptance is unavailable for this unproven external return. Exporting it does not accept it.",style=MaterialTheme.typography.caption)
    }},confirmButton={TextButton(onClick=onClose){Text("Close")}})
}
