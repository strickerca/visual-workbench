package com.visualworkbench.android.remote

import com.visualworkbench.shared.RemoteTargetBinding

/** Local cancellation never refreshes a native input grant. Access under the
 * controller inputFence. Native pause retires queued reservations/contact; this
 * fence also rejects a delayed renderer callback before state publication. */
internal class RemoteInputGrantFence {
    private var invalidated:RemoteTargetBinding?=null
    fun invalidate(binding:RemoteTargetBinding?){if(binding!=null)invalidated=binding}
    fun permits(binding:RemoteTargetBinding):Boolean = binding!=invalidated
}
