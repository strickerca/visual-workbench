package com.visualworkbench.shared

import kotlinx.coroutines.*

/** Implemented by the actual foreign producer access. No receipt import or UI
 * implementation supplies this access to the normal NativeRemoteEdit factory. */
internal interface NativeDrawingPreparationAccess {
    suspend fun prepare(expected:RemoteTargetState):NativeDrawingPreparation
}

/** Actual opaque preparation, including its source, input exclusion and helper
 * lifetime. The foreign adapter must adopt allocation before dispatcher delivery.
 * Cancellation is a nonblocking native signal. Retirement returns its actual
 * canvas state before foreign destruction and retains failed cleanup for retry. */
internal interface NativeDrawingPreparation {
    val target:RemoteTargetState
    fun cancel()
    suspend fun runConfirmed():NativeOwnedCompatibilityReceipt?
    suspend fun retire():RemoteDrawingCanvasState
}

/** Production preparation path shared by the actual session adapter and focused
 * foreign-boundary fixtures. The caller's acquisition bracket owns delivery. */
internal suspend fun prepareNativeDrawing(
    expected:RemoteTargetState,access:NativeDrawingPreparationAccess,parent:CoroutineScope,
    consumers:RemoteConsumerRegistry,current:()->Boolean,checkOpen:()->Unit,
):RemotePreparedDrawingCheck {
    checkOpen()
    if(!expected.grantActive||expected.identity.editor!=RemoteEditorKind.Paint||!expected.identity.valid())
        throw SessionFailure(SessionFailureKind.Invalid)
    if(!current())throw SessionFailure(SessionFailureKind.Closed)
    val prepared=NativeRemoteDrawingOwner(expected,access,parent,current){consumers.retired(it)}
    // Registration is the parent close fence and precedes foreign allocation.
    try{consumers.register(prepared)}catch(error:Throwable){prepared.close();throw error}
    try {
        prepared.prepare()
        currentCoroutineContext().ensureActive()
        checkOpen()
        if(!current())throw SessionFailure(SessionFailureKind.Closed)
        return prepared
    }catch(error:Throwable){
        withContext(NonCancellable){try{prepared.close()}catch(cleanup:Throwable){if(cleanup!==error)error.addSuppressed(cleanup)}}
        throw error
    }
}

/** A prepared operation escapes its acquiring coroutine while confirmation is
 * visible. The session consumer registry owns it from before allocation through
 * successful foreign retirement; a temporary coroutine borrow is insufficient.
 * All task joins and foreign calls occur outside the short state monitor. */
internal class NativeRemoteDrawingOwner(
    override val target:RemoteTargetState,
    private val access:NativeDrawingPreparationAccess,
    parent:CoroutineScope,
    private val current:()->Boolean,
    private val released:(NativeRemoteDrawingOwner)->Unit,
):RemotePreparedDrawingCheck,RemoteConsumerOwner {
    private val lock=Any()
    private val job=SupervisorJob(parent.coroutineContext[Job])
    private val scope=CoroutineScope(parent.coroutineContext.minusKey(Job)+job)
    private var invalidated=false
    private var closing=false
    private var prepared:NativeDrawingPreparation?=null
    private var preparing:Deferred<Unit>?=null
    private var running:Deferred<RemoteCompatibilityAdmission?>?=null
    private var cancellation:CompletableDeferred<Unit>?=null
    private var closeAttempt:CompletableDeferred<RemoteDrawingCanvasState>?=null
    private var retired:RemoteDrawingCanvasState?=null

    private fun usable(){
        if(invalidated||closing||retired!=null||!job.isActive||!current())
            throw SessionFailure(SessionFailureKind.Closed)
    }
    private fun matches(actual:RemoteTargetState):Boolean = actual.binding==target.binding&&
        actual.identity==target.identity&&actual.grantActive

    suspend fun prepare() {
        val task=synchronized(lock) {
            usable()
            check(preparing==null)
            scope.async(start=CoroutineStart.LAZY) {
                // A synchronous foreign constructor may finish after cancellation.
                // Publish its actual owner before any cancellable delivery or DTO.
                withContext(NonCancellable) {
                    val owned=access.prepare(target)
                    val cancelNow=synchronized(lock){prepared=owned;invalidated||closing}
                    if(cancelNow)try{owned.cancel()}catch(_:Exception){/* retirement remains authoritative */}
                    if(!matches(owned.target))throw SessionFailure(SessionFailureKind.Invalid)
                }
            }.also{preparing=it}
        }
        task.start();task.await()
        synchronized(lock){usable()}
    }

    override suspend fun runConfirmed():RemoteCompatibilityAdmission? {
        val task=synchronized(lock) {
            usable()
            check(preparing?.isCompleted==true&&running==null)
            val owned=prepared?:throw SessionFailure(SessionFailureKind.Closed)
            scope.async(start=CoroutineStart.LAZY) {
                currentCoroutineContext().ensureActive()
                synchronized(lock){usable()}
                // The actual producer enforces fresh source/tool/currentness and
                // its sequenced peer/native input exclusion before authorization.
                val receipt=owned.runConfirmed()
                currentCoroutineContext().ensureActive()
                synchronized(lock){usable()}
                receipt?.let{NativeRemoteCompatibility.admitDrawing(target,it)}
            }.also{running=it}
        }
        task.start()
        return task.await()
    }

    override fun invalidate() {
        val snapshot=synchronized(lock){
            if(invalidated)return
            invalidated=true
            cancellation=CompletableDeferred()
            Triple(preparing,running,prepared)
        }
        // Signal the actual native owner as well as its awaiting Kotlin task.
        // A failed signal cannot skip a later actual retirement/join.
        try{snapshot.third?.cancel()}catch(_:Exception){/* same owner retained */}
        finally{
            snapshot.first?.cancel();snapshot.second?.cancel()
            synchronized(lock){cancellation}?.complete(Unit)
        }
    }

    override suspend fun close():RemoteDrawingCanvasState = withContext(NonCancellable) {
        var lead=false
        val attempt=synchronized(lock) {
            retired?.let{return@withContext it}
            closeAttempt?:CompletableDeferred<RemoteDrawingCanvasState>().also {
                closing=true;closeAttempt=it;lead=true
            }
        }
        if(!lead)return@withContext attempt.await()
        try {
            invalidate()
            val tasks=synchronized(lock){preparing to running}
            // Await completion even when the original UI coroutine disappeared.
            // Preparation adopts a late owner inside its uncancellable bracket.
            tasks.first?.join();tasks.second?.join()
            // A cancellation signal acquired before close must finish before
            // retirement can destroy its foreign handle. No late cancel getter.
            synchronized(lock){cancellation}?.await()
            val owned=synchronized(lock){prepared}
            val canvas=owned?.retire()?:RemoteDrawingCanvasState.Unchanged
            synchronized(lock){prepared=null;retired=canvas}
            job.cancelAndJoin()
            released(this@NativeRemoteDrawingOwner)
            attempt.complete(canvas)
            canvas
        } catch(error:Throwable) {
            synchronized(lock){if(closeAttempt===attempt)closeAttempt=null}
            attempt.completeExceptionally(error)
            throw error
        }
    }
    override suspend fun retire():Boolean = try{close();true}catch(_:Exception){false}
}
