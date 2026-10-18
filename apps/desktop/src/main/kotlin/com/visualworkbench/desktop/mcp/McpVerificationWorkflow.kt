package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.math.BigDecimal
import java.util.Base64
import javax.imageio.ImageIO
import kotlin.math.*

/** Optional borrowed editor hook. The editor owner holds its existing edit and
 * lifecycle mutex through body and validates this exact package source revision.
 * It must never close the borrowed project or select a different document. */
internal interface McpVerificationOriginalAccess {
    /** Identity selection only; withOriginal rechecks under the editor mutex. */
    fun owns(expected:PackageInfo):Boolean
    suspend fun <T> withOriginal(expected:PackageInfo,body:suspend(WorkbenchProject)->T):T
}
/** Minted only by the native adapter from the existing capture owner's exact
 * target/frame/project delivery. It contains no portable or agent authority. */
internal class McpEditorCaptureOrigin internal constructor(
    val binding:WorkflowBinding,val sourceAssetId:String,
    internal val retain:suspend(McpCaptureEngine)->McpSelectedTarget,
)
internal data class McpVerificationReceipt(val original:VerificationPackageKey,val after:VerificationPackageKey,
    val receiptId:String,val receiptBlake3:String,val markerCount:UInt) {
    fun wire():Map<String,Any?> = mapOf("original" to original.wire(),"after" to after.wire(),
        "receipt_id" to receiptId,"receipt_blake3" to receiptBlake3,"marker_count" to markerCount.toLong())
}
private fun VerificationPackageKey.wire():Map<String,Any?> = mapOf("package_id" to packageId,"target" to target,"manifest_sha256" to manifestSha256)
internal fun PackageInfo.verificationKey()=VerificationPackageKey(packageId,target,manifestSha256)
internal data class McpMarkerComparison(val pair:VerificationPair,val number:UInt,val method:String?,
    val before:McpInboxPixels?,val after:McpInboxPixels?) {
    fun wire():Map<String,Any?> = mapOf("package_id" to pair.original.packageId,"target" to pair.original.target,
        "manifest_sha256" to pair.original.manifestSha256,"receipt_id" to pair.receiptId,"receipt_blake3" to pair.receiptBlake3,
        "n" to number.toLong(),"status" to if(method==null)"unresolved"else"aligned","method" to method,
        "before_png" to before?.let(::verificationPng),"after_png" to after?.let(::verificationPng),
        "width" to (before?.region?.width?.toLong()?:0L),"height" to (before?.region?.height?.toLong()?:0L))
}
private fun verificationPng(value:McpInboxPixels):String {
    val w=value.region.width.toInt();val h=value.region.height.toInt()
    require(w in 1..512 && h in 1..512 && value.rgbaSrgb.size==w*h*4)
    val image=BufferedImage(w,h,BufferedImage.TYPE_INT_ARGB)
    val argb=IntArray(w*h){n->val at=n*4;((value.rgbaSrgb[at+3].toInt()and 255)shl 24) or
        ((value.rgbaSrgb[at].toInt()and 255)shl 16) or ((value.rgbaSrgb[at+1].toInt()and 255)shl 8) or (value.rgbaSrgb[at+2].toInt()and 255)}
    image.setRGB(0,0,w,h,argb,0,w)
    return ByteArrayOutputStream().use{check(ImageIO.write(image,"png",it));val encoded=it.toByteArray();require(encoded.size<=2*1024*1024);Base64.getEncoder().encodeToString(encoded)}
}
private data class VerificationMarker(val number:UInt,val bounds:Rect,val point:Point,val refs:List<SemanticReference>)
private data class VerificationManifest(val info:PackageInfo,val width:UInt,val height:UInt,val scale:Double,
    val profile:McpCaptureProfile,val markers:List<VerificationMarker>)

/** Integer pixel enclosure, retaining the last covered pixel at fractional
 * marker edges. A large marker shows an explicit top-left <=512px region. */
internal fun verificationCropRegion(bounds:Rect,scale:Double,width:UInt,height:UInt):McpInboxRegion? {
    if(!scale.isFinite()||scale<=0||listOf(bounds.x,bounds.y,bounds.width,bounds.height).any{!it.isFinite()}||
        bounds.x<0||bounds.y<0||bounds.width<=0||bounds.height<=0)return null
    val x=floor(bounds.x*scale);val y=floor(bounds.y*scale)
    val endX=ceil((bounds.x+bounds.width)*scale);val endY=ceil((bounds.y+bounds.height)*scale)
    if(!endX.isFinite()||!endY.isFinite()||x<0||y<0||endX>width.toDouble()||endY>height.toDouble()||endX<=x||endY<=y)return null
    return McpInboxRegion(x.toUInt(),y.toUInt(),minOf(512.0,endX-x).toUInt(),minOf(512.0,endY-y).toUInt())
}

/** Composition of the checked catalog, real inbox, existing source owner and
 * durable native review. Coordinator serializes this under its ordinary gates;
 * no selector, note, project path or review authority crosses the agent wire. */
internal class McpVerificationWorkflow(
    private val catalog:WorkbenchPackageCatalog,private val inbox:WorkbenchMcpInbox,
    private val capture:McpCaptureEngine,private val checked:suspend(PackageInfo)->PublishedPackage,
    private val newId:()->String,
    private val verificationFactory:(WorkbenchProject,WorkbenchProject,WorkbenchPackageCatalog,WorkbenchMcpInbox)->WorkbenchVerification=::verificationWorkflows,
) {
    private data class Entry(val original:PackageInfo,val after:PackageInfo,val receipt:McpInboxReceipt,val recapture:McpVerificationRecapture) {
        val pair=VerificationPair(original.verificationKey(),after.verificationKey(),receipt.receiptId,receipt.receiptBlake3)
    }
    private val entries=linkedMapOf<String,Entry>()
    @Volatile var originalAccess:McpVerificationOriginalAccess?=null
    private suspend fun manifest(info:PackageInfo):VerificationManifest {
        checked(info)
        val bytes=catalog.readFile(info.packageId,info.target,info.manifestSha256,"manifest.json",4u*1024u*1024u)
        val parts=checkedJsonArrayParts(bytes,"markers",512)
        val root=parts.header
        fun objectAt(value:Any?):Map<*,*> = value as? Map<*,*> ?: throw McpRefused()
        fun number(value:Any?):Double = (value as? BigDecimal)?.toDouble()?.takeIf{it.isFinite()}?:throw McpRefused()
        fun rect(value:Any?):Rect {val a=value as? List<*>?:throw McpRefused();require(a.size==4);val v=a.map(::number);require(v[0]>=0&&v[1]>=0&&v[2]>0&&v[3]>0);return Rect(v[0],v[1],v[2],v[3])}
        require(root["package_id"]==info.packageId)
        val source=objectAt(root["source"]);val compiled=objectAt(root["compiled_for"])
        require(source["project_id"]==info.binding.projectId&&source["document_id"]==info.binding.documentId&&source["asset_hash"]==info.sourceAssetId&&compiled["target"]==info.target)
        fun uint(v:Any?):UInt=(v as? BigDecimal)?.longValueExact()?.takeIf{it in 1..UInt.MAX_VALUE.toLong()}?.toUInt()?:throw McpRefused()
        val width=uint(source["width"]);val height=uint(source["height"]);val edge=uint(compiled["max_long_edge"])
        val model=compiled["model"] as? String;require(model==info.model)
        val target=when(info.target){
            "generic"->PackageTarget.Generic(edge)
            "openai"->PackageTarget.OpenAiResponses(requireNotNull(model),edge)
            "gemini"->PackageTarget.Gemini(requireNotNull(model),edge)
            "claude"->when(edge){2576u->PackageTarget.ClaudeModern(requireNotNull(model));1568u->PackageTarget.ClaudeLegacy(requireNotNull(model));else->throw McpRefused()}
            else->throw McpRefused()
        }
        val scale=number(compiled["scale"]);require(scale>0&&scale<=1)
        val markers=parts.elements.map{range->val row=McpJson.decode(bytes.copyOfRange(range.first,range.last+1));val n=uint(row["number"])
            val bounds=rect(row["bbox_document"]);val p=row["point_document"] as? List<*>?:throw McpRefused();require(p.size==2)
            val refs=(row["element_refs"] as? List<*>?:throw McpRefused()).map{raw->val r=objectAt(raw)
                val platform=when(r["platform"]){"uia"->SemanticPlatform.Uia;"chromium_uia"->SemanticPlatform.ChromiumUia;"android_ax"->SemanticPlatform.AndroidAx;else->throw McpRefused()}
                SemanticReference(platform,r["eid"] as? String?:throw McpRefused(),r["name"] as? String?:throw McpRefused(),r["role"] as? String?:throw McpRefused(),r["automation_id"] as? String,r["resource_id"] as? String,r["html_id"] as? String,rect(r["bounds_document"]))
            }
            require(refs.size<=64);VerificationMarker(n,bounds,Point(number(p[0]),number(p[1])),refs)
        }
        require(markers.size.toUInt()==info.markerCount&&markers.map{it.number}==(1..markers.size).map{it.toUInt()})
        return VerificationManifest(info,width,height,scale,McpCaptureProfile(target,info.includesWindowTitle),markers)
    }
    suspend fun profile(original:PackageInfo):McpCaptureProfile=manifest(original).profile
    suspend fun remember(recapture:McpVerificationRecapture,session:String):McpVerificationReceipt {
        require(entries.size<32)
        val original=checkedIdentity(recapture.original);val after=checkedIdentity(recapture.recaptured)
        require(original.target==after.target && original.model==after.model)
        val image=after.images.single{it.role=="clean_source"}
        val png=catalog.readFile(after.packageId,after.target,after.manifestSha256,image.path,4u*1024u*1024u)
        val receipt=inbox.submit(catalog,McpInboxSubmission(newId(),original.packageId,original.target,original.manifestSha256,session,System.currentTimeMillis(),png=png))
        require(!receipt.retired&&receipt.after!=null)
        val entry=Entry(original,after,receipt,recapture)
        entries[receipt.receiptId]=entry
        return McpVerificationReceipt(entry.pair.original,entry.pair.after,receipt.receiptId,receipt.receiptBlake3,original.markerCount)
    }
    private suspend fun checkedIdentity(value:McpVerificationPackage):PackageInfo {
        val found=catalog.lookup(value.packageId,value.target,value.manifestSha256).info
        require(McpVerificationPackage.of(found)==value);checked(found);return found
    }
    private suspend fun entry(receiptId:String,receiptHash:String):Entry {
        val e=entries[receiptId]?:throw McpRefused();require(e.receipt.receiptBlake3==receiptHash)
        checked(e.original);checked(e.after)
        require(inbox.list().singleOrNull{it.receiptId==receiptId&&it.receiptBlake3==receiptHash&&!it.retired}==e.receipt)
        return e
    }
    fun has(receipt:McpInboxReceipt):Boolean=entries[receipt.receiptId]?.receipt==receipt
    private suspend fun <T> sources(e:Entry,body:suspend(WorkbenchVerification)->T):T {
        val access=originalAccess
        return if(access==null||!access.owns(e.original))capture.withVerificationSources(e.original,e.after){original,after->body(verificationFactory(original,after,catalog,inbox))}
        else access.withOriginal(e.original){original->capture.withVerificationSource(e.after){after->body(verificationFactory(original,after,catalog,inbox))}}
    }
    suspend fun review(receipt:McpInboxReceipt):VerificationReviewSnapshot {
        val e=entry(receipt.receiptId,receipt.receiptBlake3);return sources(e){it.review(e.pair)}
    }
    suspend fun change(request:VerificationOwnerChange):VerificationReviewSnapshot {
        val e=entry(request.pair.receiptId,request.pair.receiptBlake3);require(e.pair==request.pair)
        return sources(e){it.change(e.pair,request.expectedVersion,request.operationId,request.originalNumber,request.verdict,request.note)}
    }
    private suspend fun translation(e:Entry,marker:VerificationMarker,before:VerificationManifest,after:VerificationManifest,
        assumeSrgb:Boolean,allowDepth:Boolean):Pair<String,VerificationMarkerTranslation>? {
        // Paired lossless tiles share one physical pixel scale. A semantic
        // translation cannot make differently scaled crops equivalent.
        if(before.scale!=after.scale)return null
        fun measured(dx:Double,dy:Double):VerificationMarkerTranslation? {
            val x=round(dx);val y=round(dy)
            // Native pixel instructions snap source deltas to integer edges.
            // Admit only an identical source delta and paired package pixel
            // phase, rather than displaying a shift the compiler would round.
            if(!dx.isFinite()||!dy.isFinite()||abs(dx-x)>1e-7||abs(dy-y)>1e-7||
                abs(x*before.scale-round(x*before.scale))>1e-7||abs(y*before.scale-round(y*before.scale))>1e-7)return null
            return VerificationMarkerTranslation(marker.number,x,y)
        }
        val a=McpAlignmentSnapshot.fromCheckedPackage(e.original,catalog.readFile(e.original.packageId,e.original.target,e.original.manifestSha256,"semantic.json",4u*1024u*1024u))
        val b=McpAlignmentSnapshot.fromCheckedPackage(e.after,catalog.readFile(e.after.packageId,e.after.target,e.after.manifestSha256,"semantic.json",4u*1024u*1024u))
        if(a!=null&&b!=null){
            val aligned=marker.refs.mapNotNull{alignMcpMarker(e.recapture,marker.number,marker.bounds,it,a,b)}
            val offsets=aligned.map{it.afterBounds.x-it.beforeBounds.x to it.afterBounds.y-it.beforeBounds.y}.distinct()
            if(offsets.size==1){val(dx,dy)=offsets.single();return measured(dx,dy)?.let{"semantic" to it}}
        }
        if(before.scale!=after.scale||before.markers.size>64)return null
        val clean=e.receipt.before;val next=e.receipt.after?:return null
        val width=minOf(clean.width,next.width,512u);val height=minOf(clean.height,next.height,512u)
        if(width<96u||height<96u)return null
        val px=(marker.bounds.x+marker.bounds.width/2)*before.scale;val py=(marker.bounds.y+marker.bounds.height/2)*before.scale
        val x=floor(px-width.toDouble()/2).coerceIn(0.0,minOf(clean.width,next.width).toDouble()-width.toDouble()).toUInt()
        val y=floor(py-height.toDouble()/2).coerceIn(0.0,minOf(clean.height,next.height).toDouble()-height.toDouble()).toUInt()
        val region=McpInboxRegion(x,y,width,height)
        val p=inbox.pixels(e.receipt.receiptId,e.receipt.receiptBlake3,McpInboxSide.Before,region,assumeSrgb,allowDepth)
        val q=inbox.pixels(e.receipt.receiptId,e.receipt.receiptBlake3,McpInboxSide.After,region,assumeSrgb,allowDepth)
        fun changes(w:UInt,h:UInt)=before.markers.mapNotNull{marker->
            val r=marker.bounds;val left=(r.x*before.scale).coerceIn(0.0,w.toDouble());val top=(r.y*before.scale).coerceIn(0.0,h.toDouble())
            val right=((r.x+r.width)*before.scale).coerceIn(left,w.toDouble());val bottom=((r.y+r.height)*before.scale).coerceIn(top,h.toDouble())
            if(right>left&&bottom>top)Rect(left,top,right-left,bottom-top)else null
        }
        val job=currentCoroutineContext()
        val registered=registerMcpImages(p,q,changes(clean.width,clean.height),emptyList(),isCancelled={ !job.isActive })
        return if(registered is McpImageRegistrationResult.Aligned) measured(registered.dx/before.scale,registered.dy/before.scale)?.let{"registration" to it} else null
    }
    suspend fun comparison(original:VerificationPackageKey,receiptId:String,receiptHash:String,number:UInt,
        assumeSrgb:Boolean=false,allowDepth:Boolean=false):McpMarkerComparison {
        val e=entry(receiptId,receiptHash);require(e.pair.original==original)
        val before=manifest(e.original);val after=manifest(e.after);val marker=before.markers.single{it.number==number}
        val aligned=translation(e,marker,before,after,assumeSrgb,allowDepth)?:return McpMarkerComparison(e.pair,number,null,null,null)
        val(dx,dy)=aligned.second.let{it.dx to it.dy}
        val r=marker.bounds
        if(r.x+dx<0||r.y+dy<0||r.x+dx+r.width>after.width.toDouble()||r.y+dy+r.height>after.height.toDouble())return McpMarkerComparison(e.pair,number,null,null,null)
        val clean=e.receipt.before;val next=requireNotNull(e.receipt.after)
        val first=verificationCropRegion(r,before.scale,clean.width,clean.height)?:return McpMarkerComparison(e.pair,number,null,null,null)
        val second=verificationCropRegion(r.copy(x=r.x+dx,y=r.y+dy),after.scale,next.width,next.height)?:return McpMarkerComparison(e.pair,number,null,null,null)
        if(first.width!=second.width||first.height!=second.height)return McpMarkerComparison(e.pair,number,null,null,null)
        val a=inbox.pixels(receiptId,receiptHash,McpInboxSide.Before,first,assumeSrgb,allowDepth)
        val b=inbox.pixels(receiptId,receiptHash,McpInboxSide.After,second,assumeSrgb,allowDepth)
        return McpMarkerComparison(e.pair,number,aligned.first,a,b)
    }
    suspend fun compile(expected:VerificationReviewSnapshot,packageId:String,createdAtMs:Long,assumeSrgb:Boolean,allowDepth:Boolean):WorkbenchCompiledPackage? {
        val e=entry(expected.pair.receiptId,expected.pair.receiptBlake3);require(e.pair==expected.pair)
        require(expected.markers.none{it.verdict==VerificationVerdict.Pending})
        val before=manifest(e.original);val after=manifest(e.after)
        val translations=expected.markers.filter{it.verdict==VerificationVerdict.Fail}.map{review->
            currentCoroutineContext().ensureActive()
            translation(e,before.markers.single{it.number==review.originalNumber},before,after,assumeSrgb,allowDepth)?.second?:throw McpRefused()
        }
        return sources(e){it.compileFailures(expected,translations,packageId,createdAtMs)}
    }
    suspend fun checkReview(expected:VerificationReviewSnapshot){val e=entry(expected.pair.receiptId,expected.pair.receiptBlake3);require(sources(e){it.review(e.pair)}==expected)}
    fun retire(info:PackageInfo){entries.entries.removeIf{it.value.original.verificationKey()==info.verificationKey()||it.value.after.verificationKey()==info.verificationKey()}}
    fun retire(receipt:McpInboxReceipt){entries.remove(receipt.receiptId)}
    fun close(){entries.clear();originalAccess=null}
}
