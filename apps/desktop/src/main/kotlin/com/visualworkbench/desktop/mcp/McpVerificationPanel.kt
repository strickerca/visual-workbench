package com.visualworkbench.desktop.mcp

import androidx.compose.foundation.Image
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

/** Owner-only review controls. The immutable expected pair/version is captured
 * with each save; errors preserve the draft note and demand an explicit refresh. */
@Composable internal fun McpVerificationPanel(owner:DesktopMcpCoordinator,receipt:McpInboxReceipt,assumeSrgb:Boolean,allowDepth:Boolean) {
    val scope=rememberCoroutineScope()
    val interaction=remember(receipt.receiptId,receipt.receiptBlake3){VerificationInteraction()}
    var pending by remember(receipt.receiptId,receipt.receiptBlake3){mutableStateOf<VerificationOwnerChange?>(null)}
    var review by remember(receipt.receiptId,receipt.receiptBlake3){mutableStateOf<VerificationReviewSnapshot?>(null)}
    var number by remember(receipt.receiptId){mutableStateOf(1u)}
    var note by remember(receipt.receiptId){mutableStateOf("")}
    var busy by remember{mutableStateOf(false)}
    var issue by remember(receipt.receiptId){mutableStateOf<String?>(null)}
    var result by remember(receipt.receiptId){mutableStateOf<String?>(null)}
    var before by remember{mutableStateOf<InboxTile?>(null)};var after by remember{mutableStateOf<InboxTile?>(null)}
    var alignment by remember{mutableStateOf<String?>(null)}
    fun act(body:suspend()->Unit){if(busy)return;busy=true;issue=null;scope.launch{try{body()}
        catch(error:CancellationException){throw error}
        catch(_:Exception){issue="Verification refused. The exact source, receipt or review may have changed. Your draft remains; refresh before retrying."}
        finally{busy=false}}}
    fun loaded(value:VerificationReviewSnapshot){interaction.loaded(value.pair,value);review=interaction.snapshot;pending=null}
    suspend fun save(request:VerificationOwnerChange){
        val saved=owner.reviewMarker(request);interaction.changed(request,saved);review=interaction.snapshot;pending=interaction.pending
    }
    LaunchedEffect(receipt.receiptId,receipt.receiptBlake3){
        try{loaded(owner.verificationReview(receipt))}catch(error:CancellationException){throw error}
        catch(_:Exception){issue="Review needs the retained original and recapture project owners at their exact revisions. Reopen the original through its owning editor, then refresh."}
    }
    LaunchedEffect(review?.version,number){review?.markers?.singleOrNull{it.originalNumber==number}?.let{note=it.note}}
    LaunchedEffect(receipt.receiptId,receipt.receiptBlake3,number,assumeSrgb,allowDepth,review!=null){
        before=null;after=null;alignment=null
        if(review==null||number>requireNotNull(review).markerCount)return@LaunchedEffect
        var a:InboxTile?=null;var b:InboxTile?=null
        try{val pair=owner.comparison(receipt,number,assumeSrgb,allowDepth)
            if(pair.method==null){alignment="Alignment unresolved. Follow-up compilation will refuse this marker if it fails.";return@LaunchedEffect}
            a=inboxTile(requireNotNull(pair.before));b=inboxTile(requireNotNull(pair.after));currentCoroutineContext().ensureActive()
            before=a;a=null;after=b;b=null
            alignment=if(pair.method=="semantic")"Stable semantic element translation"else"Unique protected-pixel translation within the bounded search"
        }catch(error:CancellationException){throw error}
        catch(_:Exception){alignment="Aligned display unavailable. Check display permissions and exact receipt/source integrity."}
        finally{a?.close();b?.close()}
    }
    DisposableEffect(before){val image=before;onDispose{image?.close()}}
    DisposableEffect(after){val image=after;onDispose{image?.close()}}
    Divider();Text("Manual marker assessment",style=MaterialTheme.typography.h6)
    Text("Decisions and notes stay in this installation's original project store. Package output contains only failed original instructions and public lineage.",style=MaterialTheme.typography.caption)
    val current=review
    if(current!=null){
        Text("${current.markers.count{it.verdict==VerificationVerdict.Pending}} pending | ${current.markers.count{it.verdict==VerificationVerdict.Pass}} pass | ${current.markers.count{it.verdict==VerificationVerdict.Fail}} fail")
        if(current.markerCount>0u){
            Row{TextButton(enabled=!busy&&pending==null&&number>1u,onClick={number--}){Text("Previous")};Text("Original marker $number / ${current.markerCount}",Modifier.padding(top=12.dp));TextButton(enabled=!busy&&pending==null&&number<current.markerCount,onClick={number++}){Text("Next")}}
            Text("Saved verdict: ${current.markers.single{it.originalNumber==number}.verdict}")
            alignment?.let{Text(it,style=MaterialTheme.typography.caption)}
            val density=LocalDensity.current.density
            Row(Modifier.horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(12.dp)){
                before?.let{tile->Column{Text("Before marker $number");Image(tile.bitmap,"Original marker region",Modifier.size((tile.bitmap.width/density).dp,(tile.bitmap.height/density).dp),contentScale=ContentScale.None)}}
                after?.let{tile->Column{Text("After aligned region");Image(tile.bitmap,"Recaptured marker region",Modifier.size((tile.bitmap.width/density).dp,(tile.bitmap.height/density).dp),contentScale=ContentScale.None)}}
            }
            OutlinedTextField(note,{note=it.take(32768)},enabled=!busy&&pending==null,label={Text("Private owner note")},modifier=Modifier.fillMaxWidth())
            Row{VerificationVerdict.entries.forEach{verdict->TextButton(enabled=!busy&&pending==null,onClick={val marker=number;val draft=note;act{
                val request=interaction.ownerChange(owner.verificationOperationId(),marker,verdict,draft);pending=request;save(request)
            }}){Text("Save $verdict")}}}
        }else Text("The original package contains no markers to assess.")
        Button(enabled=!busy&&pending==null&&current.markers.none{it.verdict==VerificationVerdict.Pending},onClick={val expected=current;act{
            val saved=owner.compileVerificationFailures(expected,assumeSrgb,allowDepth)
            result=if(saved==null)"All pass. No follow-up package created."else"Follow-up package saved: ${saved.info.packageId}"
        }}){Text("Compile failure-only follow-up")}
    }
    pending?.let{request->TextButton(enabled=!busy,onClick={act{save(request)}}){Text("Retry exact pending save")}}
    TextButton(enabled=!busy,onClick={act{loaded(owner.verificationReview(receipt))}}){Text("Refresh saved review")}
    if(busy)LinearProgressIndicator(Modifier.fillMaxWidth())
    issue?.let{Text(it,color=MaterialTheme.colors.error)};result?.let{Text(it)}
}
