package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Test

class RemoteIntegrationReadinessTest {
    @Test fun reconnectCannotSpendPriorSyncedCarrier(){assertFalse(remoteIntegrationCarrierReady(2,5uL,5uL,true,true,true));assertTrue(remoteIntegrationCarrierReady(2,5uL,7uL,true,true,true))}
    @Test fun backgroundCanReuseCarrierButCannotRegress(){assertTrue(remoteIntegrationCarrierReady(1,5uL,5uL,true,true,true));assertFalse(remoteIntegrationCarrierReady(1,5uL,3uL,true,true,true))}
    @Test fun unavailableOrUnsettledTransportCannotAdmit(){for(epoch in listOf(0uL,2uL,4uL))assertFalse(remoteIntegrationCarrierReady(0,0uL,epoch,true,true,true));assertFalse(remoteIntegrationCarrierReady(0,0uL,1uL,false,true,true));assertFalse(remoteIntegrationCarrierReady(0,0uL,1uL,true,false,true));assertFalse(remoteIntegrationCarrierReady(0,0uL,1uL,true,true,false))}
    @Test fun freshFirstSelectionAndMissingPreviousAreDistinct(){assertTrue(remoteIntegrationCarrierReady(0,0uL,1uL,true,true,true));assertFalse(remoteIntegrationCarrierReady(0,1uL,3uL,true,true,true));assertFalse(remoteIntegrationCarrierReady(2,0uL,3uL,true,true,true))}
}
