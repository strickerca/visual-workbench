package com.visualworkbench.desktop

/** Fixture-only admission using the existing actual native carrier epoch. */
internal fun remoteIntegrationCarrierReady(index:Int,previous:ULong,current:ULong,available:Boolean,synced:Boolean,tether:Boolean):Boolean {
    if(index !in 0..2||!available||!synced||!tether||current==0uL||current and 1uL==0uL)return false
    return when(index){0->previous==0uL;1->previous>0uL&&current>=previous;else->previous>0uL&&current>previous}
}
