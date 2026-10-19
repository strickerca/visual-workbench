package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

/** Exercises the caller's explicit choice replacement against the real shared owner. */
class DesktopGiantChoiceRetirementTest {
    private class Image(private val events:MutableList<String>):GiantImage {
        override suspend fun viewport(view:GiantViewport):GiantViewportPlan=error("Unused")
        override suspend fun tile(address:GiantTileAddress,foreign:SessionResourceCharge):GiantTilePixels=error("Unused")
        override suspend fun close(){events+="image:close"}
    }
    private class Project(private val events:MutableList<String>):WorkbenchProject,WorkbenchGiantImagesProject {
        override val changes=emptyFlow<ProjectChange>()
        override suspend fun openGiantImage(binding:WorkflowBinding,diskCacheBytes:ULong,ramCacheBytes:ULong):GiantImage {
            events+="image:open";return Image(events)
        }
        override suspend fun info():ProjectInfo=error("Unused")
        override suspend fun document(documentId:String):DocumentSnapshot=error("Unused")
        override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("Unused")
        override suspend fun beginStroke(options:StrokeOptions):WorkbenchStroke=error("Unused")
        override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("Unused")
        override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("Unused")
        override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("Unused")
        override suspend fun export(options:ExportOptions):ExportResult=error("Unused")
        override suspend fun close()=Unit
    }
    private class Fixture {
        val events=mutableListOf<String>()
        val project=Project(events)
        val document=DocumentSnapshot("document","Giant",100_000u,100_000u,8u,emptyList(),RenderList(
            ProjectInfo("project","Giant","owner",8u,true,false,5u,"c".repeat(64),listOf("document")),emptyList()))
        val binding=WorkflowBinding("project","document",5u,"c".repeat(64))
        var choice=GiantDisplayChoice(binding,"a".repeat(64),GiantTileContent.Source,false)
        var close:suspend ()->Unit={}
        val owner=GiantAttachmentOwner<Pair<GiantDisplayChoice,GiantImage>>(
            attachment={DocumentAttachment(project,document,3L)},
            create={image,_,_,_,_->choice to image},
            closeView={(_,image)->events+="view:retire";close();image.close()},
            changed={},diskCacheBytes=1024u,ramCacheBytes=1024u,
            resources={object:SessionResources{override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge=error("Unused")}})
        suspend fun bind()=owner.bind(DocumentAttachment(project,document,3L))
        suspend fun change(next:GiantDisplayChoice,forceReplace:Boolean=false){
            if(giantChoiceRequiresRetirement(choice,next,forceReplace))owner.detach()
            choice=next;bind()
        }
    }

    @Test fun modeChangeJoinsTheOldOwnerBeforeOpeningTheNewImage()=runBlocking {
        val f=Fixture();f.bind();f.change(f.choice.copy(content=GiantTileContent.Composed))
        assertEquals(listOf("image:open","view:retire","image:close","image:open"),f.events)
        assertEquals(GiantTileContent.Composed,f.owner.current?.first?.content)
        f.owner.detach()
    }

    @Test fun retryOfTheSameModeRetiresTheFailedTileOwnerBeforeReopening()=runBlocking {
        val f=Fixture();f.bind();f.change(f.choice,forceReplace=true)
        assertEquals(listOf("image:open","view:retire","image:close","image:open"),f.events)
        f.owner.detach()
    }

    @Test fun canceledChoiceReplacementStillWaitsForRetirementAndNeverOpensALateImage()=runBlocking {
        val f=Fixture();f.bind()
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        f.close={entered.complete(Unit);withContext(NonCancellable){release.await()}}
        val replacement=launch(start=CoroutineStart.UNDISPATCHED){f.change(f.choice.copy(content=GiantTileContent.Composed))}
        entered.await();replacement.cancel()
        assertFalse(replacement.isCompleted)
        assertEquals(1,f.events.count{it=="image:open"})
        release.complete(Unit);replacement.join()
        assertEquals(1,f.events.count{it=="image:open"})
        assertFalse(f.owner.retaining)
    }

    @Test fun failedRetirementQuarantinesTheOldOwnerAndBlocksReplacement()=runBlocking {
        val f=Fixture();f.bind();var fail=true
        f.close={if(fail)error("retained renderer")}
        assertTrue(runCatching{f.change(f.choice.copy(content=GiantTileContent.Composed))}.isFailure)
        assertTrue(f.owner.retaining)
        assertEquals(1,f.events.count{it=="image:open"})
        fail=false;f.owner.detach()
        assertFalse(f.owner.retaining)
    }
}
