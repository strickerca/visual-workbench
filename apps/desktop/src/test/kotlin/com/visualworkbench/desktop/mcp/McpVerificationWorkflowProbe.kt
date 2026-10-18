package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.lang.reflect.Proxy

/** Synthetic checked-interface routing, not native package/capture acceptance. */
internal object McpVerificationWorkflowProbe {
    private fun id(n:Int)="019a3aaa-0123-789a-8abc-${n.toString(16).padStart(12,'0')}"
    private val original=PackageInfo(id(1),"generic",null,WorkflowBinding(id(10),id(11),7uL,"a".repeat(64)),"b".repeat(64),"c".repeat(64),"2026-10-18",128uL,1u,
        listOf(PackageImageInfo("clean","clean_source","images/clean.png",256u,256u,64uL)),true,false)
    private val after=original.copy(packageId=id(2),binding=WorkflowBinding(id(20),id(21),8uL,"d".repeat(64)),sourceAssetId="e".repeat(64),manifestSha256="f".repeat(64),markerCount=0u)
    private fun manifest(info:PackageInfo,semantic:Boolean):ByteArray {
        val marker=if(info.markerCount==0u)"" else """{"number":1,"bbox_document":[10,10,30,30],"point_document":[15,15],"element_refs":[${reference(id(30),0,0,semantic)}]}"""
        return """{"package_id":"${info.packageId}","source":{"project_id":"${info.binding.projectId}","document_id":"${info.binding.documentId}","asset_hash":"${info.sourceAssetId}","width":256,"height":256},"compiled_for":{"target":"generic","max_long_edge":2048,"scale":1},"markers":[$marker]}""".toByteArray()
    }
    private fun reference(snapshot:String,dx:Int,dy:Int,stable:Boolean)="""{"platform":"uia","eid":"$snapshot/one","name":"button","role":"button","automation_id":${if(stable)"\"stable\""else"null"},"resource_id":null,"html_id":null,"bounds_document":[${12+dx},${12+dy},24,24]}"""
    private fun semantic(info:PackageInfo,enabled:Boolean):ByteArray {
        val next=info==after;val snapshot=id(if(next)31 else 30)
        val e="""{"eid":"$snapshot/one","parent":null,"name":"button","role":"button","automation_id":"stable","resource_id":null,"html_id":null,"bounds_document":[${if(next)19 else 12},${if(next)15 else 12},24,24],"bounds_clipped":false,"text":"","enabled":true,"focused":false}"""
        return """{"schema_version":1,"text_is_untrusted":true,"project_id":"${info.binding.projectId}","document_id":"${info.binding.documentId}","snapshot_id":${if(enabled)"\"$snapshot\""else"null"},"platform":"uia","elements":[${if(enabled)e else""}]}""".toByteArray()
    }
    private class Catalog(var semantics:Boolean=true):WorkbenchPackageCatalog {
        val saved=linkedMapOf(original.packageId to PublishedPackage(original,"/synthetic/original"),after.packageId to PublishedPackage(after,"/synthetic/after"))
        val reads=mutableListOf<Pair<String,String>>()
        override suspend fun publish(packageValue:WorkbenchCompiledPackage):PublishedPackage=error("unused")
        override suspend fun list()=saved.values.toList()
        override suspend fun lookup(packageId:String,target:String,manifestSha256:String)=requireNotNull(saved[packageId]).also{require(it.info.target==target&&it.info.manifestSha256==manifestSha256)}
        override suspend fun readFile(packageId:String,target:String,manifestSha256:String,name:String,maxBytes:UInt):ByteArray {
            val info=lookup(packageId,target,manifestSha256).info;reads+=packageId to name
            return when(name){"manifest.json"->manifest(info,semantics);"semantic.json"->semantic(info,semantics);"images/clean.png"->byteArrayOf(1,2,3,4);else->error("unexpected path")}.also{require(it.size.toUInt()<=maxBytes)}
        }
        override suspend fun pendingRetirement():PackageRetirement?=null
        override suspend fun retire(packageId:String,target:String,manifestSha256:String):PackageRetirement=error("unused")
        override suspend fun close(){}
    }
    private class Inbox:WorkbenchMcpInbox {
        var submission:McpInboxSubmission?=null
        var receipt:McpInboxReceipt?=null
        val regions=mutableListOf<Pair<McpInboxSide,McpInboxRegion>>()
        override suspend fun submit(catalog:WorkbenchPackageCatalog,submission:McpInboxSubmission):McpInboxReceipt {
            this.submission=submission;val info=catalog.lookup(submission.packageId,submission.target,submission.manifestSha256).info
            val image=McpInboxImage(256u,256u,8u,64uL,"a".repeat(64),null,1u)
            return McpInboxReceipt(submission.receiptId,"9".repeat(64),info.packageId,info.target,info.manifestSha256,info.binding,info.sourceAssetId,submission.connection,submission.createdAtMs,null,"",image,image,false).also{receipt=it}
        }
        override suspend fun list()=listOfNotNull(receipt)
        override suspend fun readPng(receiptId:String,receiptBlake3:String,side:McpInboxSide)=error("unused")
        override suspend fun pixels(receiptId:String,receiptBlake3:String,side:McpInboxSide,region:McpInboxRegion,assumeUntaggedSrgb:Boolean,allowDepthReduction:Boolean):McpInboxPixels {
            require(this.receipt?.receiptId==receiptId&&this.receipt?.receiptBlake3==receiptBlake3);regions+=side to region
            return McpInboxPixels(receiptId,receiptBlake3,side,region,256u,256u,ByteArray(region.width.toInt()*region.height.toInt()*4))
        }
        override suspend fun retire(receiptId:String,receiptBlake3:String):McpInboxRetirement=error("unused")
        override suspend fun close(){}
    }
    private class Capture: McpCaptureEngine {
        var borrows=0
        private fun project():WorkbenchProject = Proxy.newProxyInstance(WorkbenchProject::class.java.classLoader,arrayOf(WorkbenchProject::class.java)){_,method,_->error("Unexpected project call ${method.name}")} as WorkbenchProject
        override suspend fun select():McpSelectedTarget=error("unused")
        override suspend fun compile(selector:String,checkGrant:suspend()->Unit):WorkbenchCompiledPackage=error("unused")
        override suspend fun<T> withVerificationSources(original:PackageInfo,after:PackageInfo,body:suspend(WorkbenchProject,WorkbenchProject)->T):T {
            require(original==McpVerificationWorkflowProbe.original&&after==McpVerificationWorkflowProbe.after);borrows++;return body(project(),project())
        }
        override suspend fun close(){}
    }
    private class Backend:WorkbenchVerification {
        var compileCalls=0;var translations:List<VerificationMarkerTranslation>?=null;var pair:VerificationPair?=null
        var budget:ULong?=null
        override suspend fun review(pair:VerificationPair)=VerificationReviewSnapshot(pair,original.binding.projectId,1u,1uL,listOf(VerificationMarkerReview(1u,VerificationVerdict.Pending,"private")))
        override suspend fun change(pair:VerificationPair,expectedVersion:ULong,operationId:String,originalNumber:UInt,verdict:VerificationVerdict,note:String):VerificationReviewSnapshot=error("unused")
        override suspend fun compileFailures(expected:VerificationReviewSnapshot,translations:List<VerificationMarkerTranslation>,packageId:String,createdAtMs:Long,memoryBudgetBytes:ULong):WorkbenchCompiledPackage? {
            compileCalls++;this.translations=translations;pair=expected.pair;budget=memoryBudgetBytes
            val failed=expected.markers.count{it.verdict==VerificationVerdict.Fail}.toUInt()
            if(failed==0u)return null
            // This records the real Desktop -> shared default argument, not
            // native compile evidence. The normal native compiler alone
            // reserves 2 * 32 MiB package bytes + 96 MiB before codec work.
            // Even before canonical/source costs, the old128MiB cannot fit.
            if(memoryBudgetBytes<=2uL*32uL*1024uL*1024uL+96uL*1024uL*1024uL)
                throw PackageFailure(PackageFailureKind.Limit)
            check(memoryBudgetBytes<=256uL*1024uL*1024uL)
            return object:WorkbenchCompiledPackage {
                override val info=after.copy(packageId=packageId,markerCount=failed)
                override suspend fun close(){}
            }
        }
    }
    private suspend fun refused(body:suspend()->Unit){var rejected=false;try{body()}catch(_:IllegalArgumentException){rejected=true}catch(_:IllegalStateException){rejected=true}catch(_:McpRefused){rejected=true}catch(_:PackageFailure){rejected=true};check(rejected)}
    @JvmStatic fun main(args:Array<String>)=runBlocking {
        check(verificationCropRegion(Rect(0.5,1.5,10.0,10.0),1.0,32u,32u)==McpInboxRegion(0u,1u,11u,11u))
        check(verificationCropRegion(Rect(500.0,500.0,1000.0,1000.0),1.0,2048u,2048u)==McpInboxRegion(500u,500u,512u,512u))
        check(verificationCropRegion(Rect(31.5,1.0,1.0,1.0),1.0,32u,32u)==null)
        println("PASS fractional edges include the final covered pixel and explicit tile bounds")
        val catalog=Catalog();val inbox=Inbox();val capture=Capture();val backend=Backend()
        val workflow=McpVerificationWorkflow(catalog,inbox,capture,{catalog.lookup(it.packageId,it.target,it.manifestSha256)},{id(100)},{_,_,_,_->backend})
        val relation=McpVerificationRecapture(McpVerificationPackage.of(original),McpVerificationPackage.of(after))
        val result=workflow.remember(relation,"a".repeat(32));val receipt=requireNotNull(inbox.receipt)
        check(result.original==original.verificationKey()&&result.after==after.verificationKey()&&result.receiptId==id(100))
        check(requireNotNull(inbox.submission).png.contentEquals(byteArrayOf(1,2,3,4))&&catalog.reads.last()==(after.packageId to "images/clean.png"))
        println("PASS real-after catalog route supplies comparison bytes and core receipt identity")
        val profile=workflow.profile(original);check(profile.target==PackageTarget.Generic(2048u));println("PASS original target profile read through checked manifest")
        val pair=workflow.comparison(original.verificationKey(),receipt.receiptId,receipt.receiptBlake3,1u)
        check(pair.method=="semantic"&&pair.before?.region==McpInboxRegion(10u,10u,30u,30u)&&pair.after?.region==McpInboxRegion(17u,13u,30u,30u))
        val wire=pair.wire();check(wire.keys.none{it.contains("note")||it.contains("selector")||it.contains("verdict")});println("PASS semantic translation yields exact paired crops and private-free wire")
        refused{workflow.comparison(original.verificationKey(),receipt.receiptId,"0".repeat(64),1u)};println("PASS changed receipt hash refuses")
        refused{workflow.review(receipt.copy(receiptId=id(101)))};println("PASS unrelated agent inbox receipt has no recapture authority")
        val review=workflow.review(receipt);check(capture.borrows==1&&review.pair.receiptId==receipt.receiptId);println("PASS review borrows both retained sources")
        workflow.originalAccess=object:McpVerificationOriginalAccess {
            override fun owns(expected:PackageInfo)=false
            override suspend fun<T> withOriginal(expected:PackageInfo,body:suspend(WorkbenchProject)->T):T=error("unrelated editor must not borrow")
        }
        workflow.review(receipt);check(capture.borrows==2);println("PASS unrelated installed editor hook preserves retained engine originals")
        workflow.originalAccess=object:McpVerificationOriginalAccess {
            override fun owns(expected:PackageInfo)=true
            override suspend fun<T> withOriginal(expected:PackageInfo,body:suspend(WorkbenchProject)->T):T=throw PackageFailure(PackageFailureKind.Stale)
        }
        refused{workflow.review(receipt)};check(capture.borrows==2);workflow.originalAccess=null
        println("PASS stale matching editor source never falls back to another owner")
        refused{workflow.compile(review,id(200),0,false,false)};check(backend.compileCalls==0);println("PASS pending review never reaches failure compiler")
        val passed=review.copy(markers=listOf(VerificationMarkerReview(1u,VerificationVerdict.Pass,"private")))
        check(workflow.compile(passed,id(200),0,false,false)==null&&backend.compileCalls==1&&backend.translations==emptyList<VerificationMarkerTranslation>());println("PASS all-pass still reaches authoritative native currentness check")
        val failed=review.copy(markers=listOf(VerificationMarkerReview(1u,VerificationVerdict.Fail,"private")))
        val followUp=requireNotNull(workflow.compile(failed,id(201),0,false,false))
        try {
            check(followUp.info.packageId==id(201)&&followUp.info.markerCount==1u&&followUp.info.binding==after.binding)
            check(backend.compileCalls==2&&backend.pair==review.pair&&backend.budget==256uL*1024uL*1024uL)
            check(backend.translations==listOf(VerificationMarkerTranslation(1u,7.0,3.0)))
        }finally{followUp.close()}
        println("PASS nonempty failure follows the actual Desktop/shared default budget within the native ceiling")
        catalog.semantics=false
        val unresolved=workflow.comparison(original.verificationKey(),receipt.receiptId,receipt.receiptBlake3,1u)
        check(unresolved.method==null&&unresolved.before==null&&unresolved.after==null)
        refused{workflow.compile(review.copy(markers=listOf(VerificationMarkerReview(1u,VerificationVerdict.Fail,""))),id(200),0,false,false)}
        check(backend.compileCalls==2);println("PASS uniform equal-size images do not invent alignment or compile failures")
        workflow.retire(after);refused{workflow.review(receipt)};println("PASS after retirement invalidates pair")
        println("14 synthetic workflow-routing cases passed; native, app, OS and device gates remain unrun.")
    }
}
