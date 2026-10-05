package com.visualworkbench.shared

/** Phone-local admission sample to matching validated render callback. This
 * reports neither compositor photons nor completion of an editor paint. */
public data class RemoteEchoTiming(
    public val binding:RemoteTargetBinding?,public val observed:ULong,
    public val missing:ULong,public val pending:Int,public val p50Nanos:Long?,
    public val p95Nanos:Long?,public val lastFrameId:ULong,public val lastTicket:ULong,
)
/** Caller owns one short local fence; no provider or native work occurs here. */
public class RemoteEchoLedger {
    private var binding:RemoteTargetBinding?=null
    private val pending=linkedMapOf<ULong,Long>()
    private val measured=mutableListOf<Long>()
    private var observed=0uL;private var missing=0uL
    private var frame=0uL;private var ticket=0uL;private var clock=0L
    public fun record(admission:RemoteInputAdmission):Boolean {
        if(admission.sequence==0uL||admission.sampledLocalNanos<0)return false
        if(binding!=admission.binding){binding=admission.binding;pending.clear();measured.clear();observed=0uL;missing=0uL;frame=0uL;ticket=0uL;clock=0L}
        if(pending.containsKey(admission.sequence))return false
        if(pending.size>=256){pending.remove(pending.keys.first());missing++}
        pending[admission.sequence]=admission.sampledLocalNanos;return true
    }
    public fun acknowledge(receipt:RemoteRenderedAcknowledgment,callbackNanos:Long):Boolean {
        if(receipt.binding!=binding||receipt.frameId<=frame||receipt.ticket==ticket||callbackNanos<clock)return false
        if(pending.any{(seq,sampled)->seq<=receipt.lastInputSequenceApplied&&sampled>callbackNanos})return false
        expire(callbackNanos)
        val covered=pending.filterKeys{it<=receipt.lastInputSequenceApplied}
        for((seq,sampled)in covered){if(measured.size>=256)measured.removeAt(0);measured+=callbackNanos-sampled;pending.remove(seq);observed++}
        frame=receipt.frameId;ticket=receipt.ticket;return true
    }
    private fun expire(now:Long){if(now<clock)return;clock=now;val expired=pending.filterValues{now>=it&&now-it>=500_000_000L}.keys;for(seq in expired){pending.remove(seq);missing++}}
    public fun retire():Unit{missing+=pending.size.toULong();pending.clear()}
    public fun snapshot(now:Long):RemoteEchoTiming {
        expire(now);val ordered=measured.sorted()
        fun percentile(n:Int):Long?=if(ordered.isEmpty())null else ordered[((ordered.size*n+99)/100-1).coerceAtLeast(0)]
        return RemoteEchoTiming(binding,observed,missing,pending.size,percentile(50),percentile(95),frame,ticket)
    }
}
