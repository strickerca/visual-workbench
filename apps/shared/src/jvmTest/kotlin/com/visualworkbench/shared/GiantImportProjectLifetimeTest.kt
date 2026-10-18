package com.visualworkbench.shared

import java.nio.file.Files
import java.util.Base64
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

/** Final-ABI caller test: borrowing the actual project pins its foreign handle
 * through joined work, including ordinary projects created before giant intake. */
class GiantImportProjectLifetimeTest {
    @Test fun projectCloseWaitsForGiantBorrowAndClosedProjectIsTypedRefusal() = runBlocking {
        val root = Files.createTempDirectory("vw-giant-project-lifetime-")
        val core = workbenchCore()
        val now = 1_700_000_000_000L
        val bytes = Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQ1QoBAADrAJRe4xMbAAAAAElFTkSuQmCC")
        val project = core.create(CreateProject(root.resolve("project").toString(), core.newId(now.toULong()),
            core.newId((now + 1).toULong()), core.newId((now + 2).toULong()), core.newDeviceId(),
            "Synthetic lifetime", bytes, now))
        val entered = CompletableDeferred<Unit>(); val leave = CompletableDeferred<Unit>()
        val borrow = launch(start = CoroutineStart.UNDISPATCHED) {
            withNativeGiantProject(project) { entered.complete(Unit); leave.await() }
        }
        entered.await()
        val close = launch(start = CoroutineStart.UNDISPATCHED) { project.close() }
        try {
            assertNull(withTimeoutOrNull(100) { close.join(); true })
            assertFalse(close.isCompleted)
            assertEquals("Synthetic lifetime", project.info().title)
            leave.complete(Unit)
            withTimeout(5000) { borrow.join(); close.join() }
            val failure = runCatching { withNativeGiantProject(project) { error("closed handle escaped") } }.exceptionOrNull()
            assertEquals(GiantImageFailureKind.Closed, (failure as GiantImageFailure).kind)
        } finally {
            leave.complete(Unit)
            withContext(NonCancellable) { borrow.cancelAndJoin(); close.join(); project.close() }
            // A failed close exits before deleting any synthetic project files.
            check(root.toFile().deleteRecursively())
        }
    }
}
