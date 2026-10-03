package com.visualworkbench.shared
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.nio.file.Files
import kotlinx.coroutines.runBlocking
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
public class AndroidCoreSmokeTest {
    @Test public fun rustGoldenAndBoundary(): Unit = runBlocking {
        val cache=InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.toPath()
        val directory=Files.createTempDirectory(cache,"vw-ffi-smoke-").toFile()
        try { CoreSmokeContract.golden(directory); CoreSmokeContract.hundredThousand(directory); CoreSmokeContract.cancelledHandles(directory); CoreSmokeContract.cancelledStrokeDisposal(directory); CoreSmokeContract.cancelledSessionOwnership(); CoreSmokeContract.cancelledProjectClosure(directory) }
        finally { drainNativeTransfers(); CoreSmokeContract.disposeDirectory(directory) }
    }
}
