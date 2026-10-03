package com.visualworkbench.shared

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow

public abstract class WorkbenchViewModel(dispatcher: CoroutineDispatcher = Dispatchers.Default) {
    protected val scope: CoroutineScope = CoroutineScope(SupervisorJob() + dispatcher)
    public open fun dispose(): Unit = scope.cancel()
}
/** Bounded, serial commands. A full queue is explicit, never a dropped edit. */
public class CommandDispatcher(private val scope: CoroutineScope, capacity: Int = 32) {
    private val queue: Channel<suspend () -> Unit> = Channel(capacity)
    private val errorsMutable: MutableSharedFlow<Throwable> = MutableSharedFlow(extraBufferCapacity = 1)
    public val errors: SharedFlow<Throwable> = errorsMutable
    init { scope.launch { for (command in queue) { try { command() } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel } catch (error: Exception) { errorsMutable.emit(error) } } } }
    public fun offer(command: suspend () -> Unit): Boolean = queue.trySend(command).isSuccess
    public fun close(): Boolean = queue.close()
}
public object DesignTokens {
    public const val MinimumTouchTargetDp: Int = 48
    public const val PanelSpacingDp: Int = 12
    public const val CompactSpacingDp: Int = 8
    public const val BackgroundArgb: UInt = 0xff121820u
    public const val ForegroundArgb: UInt = 0xffeef2f6u
    public const val AccentArgb: UInt = 0xff73d7c1u
}
