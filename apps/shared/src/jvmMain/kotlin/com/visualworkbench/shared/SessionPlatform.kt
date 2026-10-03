package com.visualworkbench.shared

import com.visualworkbench.bindings.core.SessionException
import com.visualworkbench.bindings.core.validateSessionEndpoint
import java.net.Inet6Address
import java.net.NetworkInterface
import java.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

private const val QR_PREFIX="vw-pair:v1:"
private const val MAX_QR_BYTES=4096
private val MAX_QR_TEXT=QR_PREFIX.length+(MAX_QR_BYTES*4+2)/3
private val LOCAL_ENUMERATION=Semaphore(2)

public actual fun encodePairingQr(qr: ByteArray): String {
    if(qr.isEmpty()||qr.size>MAX_QR_BYTES) throw SessionFailure(SessionFailureKind.Invalid)
    return QR_PREFIX+Base64.getUrlEncoder().withoutPadding().encodeToString(qr)
}
public actual fun decodePairingQr(text: String): ByteArray {
    if(text.length>MAX_QR_TEXT||!text.startsWith(QR_PREFIX)) throw SessionFailure(SessionFailureKind.Invalid)
    val body=text.substring(QR_PREFIX.length)
    if(body.isEmpty()||body.any { it !in 'A'..'Z' && it !in 'a'..'z' && it !in '0'..'9' && it!='-' && it!='_' }) throw SessionFailure(SessionFailureKind.Invalid)
    val decoded=try { Base64.getUrlDecoder().decode(body) } catch(_: IllegalArgumentException) { throw SessionFailure(SessionFailureKind.Invalid) }
    if(decoded.size>MAX_QR_BYTES||Base64.getUrlEncoder().withoutPadding().encodeToString(decoded)!=body) { decoded.fill(0);throw SessionFailure(SessionFailureKind.Invalid) }
    return decoded
}

public actual suspend fun localSessionAddresses(): List<LocalSessionAddress> = LOCAL_ENUMERATION.withPermit { withContext(Dispatchers.IO) {
    val result=mutableListOf<LocalSessionAddress>()
    try {
        val interfaces=NetworkInterface.getNetworkInterfaces() ?: return@withContext emptyList()
        var count=0
        while(interfaces.hasMoreElements()) {
            if(++count>256) throw SessionFailure(SessionFailureKind.Backpressure)
            val network=interfaces.nextElement()
            if(network.index<=0||!network.isUp) continue
            val addresses=network.inetAddresses
            var addressCount=0
            while(addresses.hasMoreElements()) {
                if(++addressCount>64) throw SessionFailure(SessionFailureKind.Backpressure)
                val address=addresses.nextElement()
                val raw=address.hostAddress?.substringBefore('%') ?: continue
                val numeric=if(address is Inet6Address && address.isLinkLocalAddress) "$raw%${network.index}" else raw
                val endpoint=if(address is Inet6Address) "[$numeric]:0" else "$numeric:0"
                try { validateSessionEndpoint(endpoint,true) } catch(_: SessionException.Invalid) { continue }
                if(result.size>=256) throw SessionFailure(SessionFailureKind.Backpressure)
                result.add(LocalSessionAddress(numeric,network.index.toUInt()))
            }
        }
    } catch(error: SessionFailure) { throw error }
    catch(error: SessionException) { throw sessionFailure(error) }
    catch(_: java.net.SocketException) { throw SessionFailure(SessionFailureKind.Transport) }
    catch(_: SecurityException) { throw SessionFailure(SessionFailureKind.Transport) }
    result.distinct().sortedWith(compareBy({it.interfaceIndex},{it.address}))
} }
