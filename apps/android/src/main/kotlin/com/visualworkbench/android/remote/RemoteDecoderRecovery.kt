package com.visualworkbench.android.remote

import kotlinx.coroutines.delay

internal enum class RemoteRecoveryRetirement { Complete, Pending, Obsolete }

/** A decoder's existing deadline has already requested recovery. Keep its exact
 * owner/ticket while actual retirement is Pending; no new render budget starts.
 * An obsolete worker cannot retire or request recovery for a replacement owner. */
internal suspend fun retireRemoteDecoderRecovery(
    retire: suspend () -> RemoteRecoveryRetirement,
    request: suspend () -> Unit,
    pending: suspend () -> Unit = { delay(20) },
) {
    while (true) {
        when (retire()) {
            RemoteRecoveryRetirement.Complete -> { request(); return }
            RemoteRecoveryRetirement.Obsolete -> return
            RemoteRecoveryRetirement.Pending -> pending()
        }
    }
}
