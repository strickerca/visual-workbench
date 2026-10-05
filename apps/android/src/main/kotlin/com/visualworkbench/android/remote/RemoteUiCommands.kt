package com.visualworkbench.android.remote

import com.visualworkbench.shared.RemoteEditState
import com.visualworkbench.shared.RemoteEditStatus
import com.visualworkbench.shared.SessionFailure
import com.visualworkbench.shared.SessionFailureKind
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import com.visualworkbench.shared.RemoteVideoScope

internal enum class RemoteUiCommand { RequestControl,Pause }
internal fun remoteUiCommandEnabled(command:RemoteUiCommand,state:RemoteEditState):Boolean {
    if(state.scope==null)return false
    return when(command) {
        RemoteUiCommand.RequestControl->state.status in setOf(RemoteEditStatus.Viewing,RemoteEditStatus.Paused)
        RemoteUiCommand.Pause->state.status in setOf(RemoteEditStatus.Viewing,RemoteEditStatus.PendingFocus,RemoteEditStatus.PendingGrant,RemoteEditStatus.Controlling)
    }
}
/** Admission is checked again at execution. Native current scope/grant remains
 * authoritative if disconnect or selection changes after this UI fence. */
internal suspend fun runRemoteUiCommand(command:RemoteUiCommand,state:()->RemoteEditState,
    closing:()->Boolean,call:suspend()->Unit):String? {
    if(closing()||!remoteUiCommandEnabled(command,state()))return "Select an available PC window before requesting control."
    return try{call();null}
    catch(cancelled:CancellationException){throw cancelled}
    catch(refused:SessionFailure){when(refused.kind){
        SessionFailureKind.RemoteRetirementPending->"Retirement is pending; keep this view open."
        SessionFailureKind.RemotePartialInput->"Control is sealed after partial input; revalidate it on the PC."
        SessionFailureKind.Closed,SessionFailureKind.Transport->"The link changed; reconnect and reselect the PC window."
        else->"The PC refused this command; check its selected window and grant."
    }}
    catch(_:Exception){"The command failed; no retry was sent."}
}

/** A UI-only admission token is revoked on every background/resume/close/pause.
 * It never supplies a native grant. Same-main-thread lifecycle and callback checks
 * fence a delayed request against activity and exact selected video scope. */
internal class RemoteUiInputAdmission {
    internal class Ticket(val generation:Any,val scope:RemoteVideoScope)
    private var generation:Any=Any()
    @Synchronized fun capture(scope:RemoteVideoScope?):Ticket?=scope?.let{Ticket(generation,it)}
    @Synchronized fun revoke(){generation=Any()}
    @Synchronized fun resume(ticket:Ticket?,state:()->RemoteEditState,closing:()->Boolean,
        resumed:()->Boolean,callback:()->Unit):Boolean {
        if(ticket==null||ticket.generation!==generation||closing()||!resumed())return false
        val current=state()
        if(current.scope!=ticket.scope||current.status !in setOf(RemoteEditStatus.Viewing,
            RemoteEditStatus.PendingFocus,RemoteEditStatus.PendingGrant,RemoteEditStatus.Controlling,RemoteEditStatus.Paused))return false
        callback();return true
    }
}
internal suspend fun runRemoteUiCommandWithAdmission(command:RemoteUiCommand,
    admission:RemoteUiInputAdmission,ticket:RemoteUiInputAdmission.Ticket?,state:()->RemoteEditState,
    closing:()->Boolean,resumed:()->Boolean,call:suspend()->Unit,resume:()->Unit):String? {
    val refused=runRemoteUiCommand(command,state,closing,call)
    if(refused!=null)return refused
    currentCoroutineContext().ensureActive()
    if(command==RemoteUiCommand.RequestControl&&!admission.resume(ticket,state,closing,resumed,resume))
        return "Control request sent; input stays paused while this view or the selected window changes."
    return null
}
