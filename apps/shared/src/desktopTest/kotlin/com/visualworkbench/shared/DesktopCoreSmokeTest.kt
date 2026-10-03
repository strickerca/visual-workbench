package com.visualworkbench.shared
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import org.junit.Test

public class DesktopCoreSmokeTest {
    @Test public fun rustGoldenAndBoundary(): Unit = runBlocking {
        val directory=Files.createTempDirectory("vw-ffi-smoke-").toFile()
        try { CoreSmokeContract.golden(directory); CoreSmokeContract.hundredThousand(directory); CoreSmokeContract.cancelledHandles(directory); CoreSmokeContract.cancelledStrokeDisposal(directory); CoreSmokeContract.cancelledSessionOwnership(); CoreSmokeContract.cancelledProjectClosure(directory) }
        finally { drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory) }
    }
}
