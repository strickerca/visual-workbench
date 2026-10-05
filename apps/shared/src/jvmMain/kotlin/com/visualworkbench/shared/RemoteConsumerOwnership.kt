package com.visualworkbench.shared

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.delay

/** A bounded atomic admission/close fence. Pending consumers remain strongly
 * owned until their actual retirement returns success. */
internal class RemoteConsumerRegistry {
    private val owners=linkedSetOf<RemoteConsumerOwner>()
    private var closing=false
    @Synchronized fun register(owner:RemoteConsumerOwner){if(closing)throw SessionFailure(SessionFailureKind.Closed);require(owners.size<4);owners.add(owner)}
    @Synchronized fun snapshot():List<RemoteConsumerOwner> = owners.toList()
    @Synchronized fun fence():List<RemoteConsumerOwner>{closing=true;return owners.toList()}
    @Synchronized fun retired(owner:RemoteConsumerOwner){owners.remove(owner)}
    fun invalidate(){for(owner in snapshot())try{owner.invalidate()}catch(_:Exception){/* actual close still retains and retires this owner */}}
}
/** Parse and delivery failures discharge exactly the acquired native ticket.
 * Cleanup is awaited under cancellation; no rendered acknowledgement is sent. */
internal suspend fun <T> ownRemoteTicket(ticket:ULong,discard:suspend(ULong)->Unit,use:suspend()->T):T {
    try{return use()}catch(error:Throwable){
        withContext(NonCancellable){try{discard(ticket)}catch(cleanup:Throwable){error.addSuppressed(cleanup)}}
        throw error
    }
}

/** A consumed/malformed receipt still owns the grant's retirement. Primary
 * protocol failures and cancellation survive cleanup failures via suppression. */
internal suspend fun <T> ownRemoteCommandAttempt(block:suspend()->T,retire:suspend()->Unit):T {
    var primary:Throwable?=null
    try{return block()}catch(error:Throwable){primary=error;throw error}
    finally{withContext(NonCancellable){
        // Keep the return to the caller on its dispatcher non-cancellable;
        // dispatch only the actual native cleanup work to the worker.
        withContext(Dispatchers.Default){
            try{retire()}catch(cleanup:Throwable){
                val original=primary
                if(original==null)throw cleanup
                if(original!==cleanup)original.addSuppressed(cleanup)
            }
        }
    }}
}
/** Pause is unconditional and precedes cancel. A missing cancel ticket or a
 * failed pause cannot skip real producer retirement observation. */
internal suspend fun retireIncompleteRemoteCommand(receiptTaken:Boolean,pause:suspend()->Unit,cancel:suspend()->Unit,retired:suspend()->Boolean) {
    var failure:Throwable?=null
    fun failed(error:Throwable){val prior=failure;if(prior==null)failure=error else if(prior!==error)prior.addSuppressed(error)}
    try{pause()}catch(error:Throwable){failed(error)}
    if(!receiptTaken)try{cancel()}catch(error:Throwable){failed(error)}
    val started=System.nanoTime()
    var actualRetired=false
    try{
        actualRetired=retired()
        while(!actualRetired&&System.nanoTime()-started<500_000_000L){delay(5);actualRetired=retired()}
    }catch(error:Throwable){failed(error)}
    if(!actualRetired){
        val pending=SessionFailure(SessionFailureKind.RemoteRetirementPending)
        failure?.let{pending.addSuppressed(it)}
        throw pending
    }
    failure?.let{throw it}
}

/** Native FFI DTO acceptance fields are checked before completion. Transport
 * correlation has already been checked natively and is never a manufactured ACK. */
internal fun checkedRemoteCommandReceipt(binding:RemoteTargetBinding,action:RemoteEditorAction,statusText:String,sequence:ULong,qpc100ns:ULong,reason:String?):RemoteCommandReceipt {
    val status=when(statusText){"injected"->RemoteCommandStatus.Injected;"sealed_partial"->RemoteCommandStatus.SealedPartial;"refused"->RemoteCommandStatus.Refused;else->error("Unknown remote receipt")}
    if(status==RemoteCommandStatus.Injected){require(sequence>0uL&&qpc100ns>0uL&&reason.isNullOrEmpty())}
    else {require(sequence==0uL&&qpc100ns==0uL)}
    return RemoteCommandReceipt(binding,action,sequence,qpc100ns,status,reason)
}
