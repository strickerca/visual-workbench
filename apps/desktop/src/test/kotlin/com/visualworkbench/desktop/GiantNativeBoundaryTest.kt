package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.util.Comparator
import javax.imageio.ImageIO

/** Real generated Kotlin/native error and charge boundary. Tiny owned synthetic
 * sources do not establish giant decoding, a producer, GPU or device acceptance. */
class GiantNativeBoundaryTest {
    @Test fun foreignChargeReleasesIdempotentlyAndMapsNativePoolRefusal()=runBlocking {
        val resources=sessionResources();val charge=resources.reserveForeign(16uL)
        charge.close();charge.close()
        val failure=runCatching{resources.reserveForeign(ULong.MAX_VALUE)}.exceptionOrNull()
        assertTrue(failure is GiantImageFailure)
        assertEquals(GiantImageFailureKind.Memory,(failure as GiantImageFailure).kind)
        assertNotNull(failure.budgetBytes)
    }
    @Test fun generatedProviderRefusesMissingCacheStaleBindingAndClosedProject()=runBlocking {
        val root=Files.createTempDirectory("vw-giant-native-")
        var project:WorkbenchProject?=null
        try {
            val core=workbenchCore();val now=System.currentTimeMillis()
            val source=ByteArrayOutputStream().also{check(ImageIO.write(BufferedImage(2,2,BufferedImage.TYPE_INT_RGB),"png",it))}.toByteArray()
            val opened=core.create(CreateProject(root.resolve("project").toString(),core.newId(now.toULong()),
                core.newId(now.toULong()),core.newId(now.toULong()),core.newDeviceId(),"Synthetic boundary",source,now))
            project=opened;val before=opened.info();val document=before.documentIds.single()
            val binding=WorkflowBinding(before.projectId,document,before.hostSeq,before.stateHash)
            val images=opened as WorkbenchGiantImagesProject
            suspend fun refused(bound:WorkflowBinding,kind:GiantImageFailureKind){
                val failure=runCatching{images.openGiantImage(bound,64uL*1024uL*1024uL,16uL*1024uL*1024uL)}.exceptionOrNull()
                assertTrue(failure is GiantImageFailure);assertEquals("lengths=${bound.projectId.length}/${bound.documentId.length}/${bound.stateHash.length}; source=${source.size}",kind,(failure as GiantImageFailure).kind)
            }
            refused(binding,GiantImageFailureKind.Missing)
            refused(binding.copy(stateHash=if(binding.stateHash=="b".repeat(64))"a".repeat(64)else"b".repeat(64)),GiantImageFailureKind.Stale)
            val after=opened.info();assertEquals(before.hostSeq,after.hostSeq);assertEquals(before.stateHash,after.stateHash)
            opened.close();project=null;refused(binding,GiantImageFailureKind.Closed)
        } finally {
            project?.close()
            Files.walk(root).use{paths->paths.sorted(Comparator.reverseOrder()).forEach{Files.delete(it)}}
        }
    }
}
