package com.visualworkbench.desktop.mcp

import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test

/** Synthetic temporary trees only; never touches a user's cache or starts the runtime. */
class McpRuntimeVerifyTest {
    private val required=listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs","vw-codex-host.exe","mcp/codex/main.mjs","mcp/codex/client.mjs","mcp/codex/package.mjs","mcp/codex/process.mjs","mcp/codex/schema.mjs","mcp/codex/schema-worker.mjs","mcp/codex/image-profiles.json")
    private val windows=System.getProperty("os.name").startsWith("Windows",ignoreCase=true)
    private fun sha(bytes:ByteArray)=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}
    private fun fails(body:()->Unit){try{body();fail("Expected refusal")}catch(_:IllegalArgumentException){}catch(_:McpRefused){}}
    private class Tree(val root:Path,val hashes:LinkedHashMap<String,String>)
    private fun tree(extra:List<String> = emptyList()):Tree {
        val root=Files.createTempDirectory("vw-mcp-verify-").toRealPath()
        val hashes=LinkedHashMap<String,String>()
        for(name in required+extra){
            val bytes="content:$name".toByteArray()
            val path=root.resolve(name);Files.createDirectories(path.parent);Files.write(path,bytes);hashes[name]=sha(bytes)
        }
        return Tree(root,hashes)
    }
    /** Removes a reparse point itself and never descends through it. */
    private fun deleteFixture(root:Path){Files.walkFileTree(root,object:SimpleFileVisitor<Path>(){
        override fun preVisitDirectory(dir:Path,attrs:BasicFileAttributes):FileVisitResult=
            if(attrs.isOther && dir!=root){Files.delete(dir);FileVisitResult.SKIP_SUBTREE}else FileVisitResult.CONTINUE
        override fun visitFile(file:Path,attrs:BasicFileAttributes):FileVisitResult{Files.delete(file);return FileVisitResult.CONTINUE}
        override fun postVisitDirectory(dir:Path,exc:java.io.IOException?):FileVisitResult{if(exc!=null)throw exc;Files.delete(dir);return FileVisitResult.CONTINUE}
    })}
    private fun <T> Tree.use(body:(Tree)->T):T=try{body(this)}finally{deleteFixture(root)}
    private fun Tree.verify(progress:McpStartupProgress?=null)=McpRuntime.verify(root,hashes,serverOnly=true,progress=progress)

    @Test fun exactTreeIsAdmittedAndEveryListedByteIsHashedInManifestOrder()=tree(listOf("mcp/node_modules/a/b/index.js")).use{t->
        val progress=McpStartupProgress{0L};progress.begin();progress.inventory(t.hashes.size,t.hashes.keys.sumOf{"content:$it".length.toLong()})
        assertEquals(t.root,t.verify(progress).root)
        assertEquals(t.hashes.size,progress.snapshot().verifiedFiles)
        assertEquals(progress.snapshot().bytes,progress.snapshot().verifiedBytes)
        assertEquals(t.hashes.keys,McpRuntime.storedFiles(t.root))
    }

    @Test fun everyStartRehashesSoAChangedByteIsRefusedAfterAPriorAcceptance()=tree().use{t->
        t.verify()
        Files.write(t.root.resolve("mcp/codex/schema.mjs"),"content:mcp/codex/schema.mjX".toByteArray())
        val progress=McpStartupProgress{0L};progress.begin();progress.inventory(t.hashes.size,1_000)
        fails{t.verify(progress)}
        assertEquals(required.indexOf("mcp/codex/schema.mjs"),progress.snapshot().verifiedFiles)
        // A same-size forgery of a large executable is refused the same way.
        Files.write(t.root.resolve("mcp/codex/schema.mjs"),"content:mcp/codex/schema.mjs".toByteArray());t.verify()
        Files.write(t.root.resolve("node.exe"),"content:node.exX".toByteArray());fails{t.verify()}
    }

    @Test fun unlistedOrMissingFilesAreRefusedEvenWhenListedHashesMatch()=tree().use{t->
        Files.createDirectories(t.root.resolve("mcp/node_modules/x"));Files.write(t.root.resolve("mcp/node_modules/x/index.js"),byteArrayOf(1))
        fails{t.verify()}
        Files.delete(t.root.resolve("mcp/node_modules/x/index.js"));t.verify() // An empty directory carries no code.
        Files.delete(t.root.resolve("mcp/codex/client.mjs"));fails{t.verify()}
    }

    @Test fun manifestNameMustEqualTheStoredNameExactly()=tree(listOf("lib/Data.js")).use{t->
        t.verify()
        val variant=LinkedHashMap<String,String>();t.hashes.forEach{(k,v)->variant[if(k=="lib/Data.js")"lib/data.js" else k]=v}
        // On a case-insensitive volume this path opens the same bytes; the
        // stored-name inventory still refuses the alias.
        fails{McpRuntime.verify(t.root,variant,serverOnly=true)}
        val dirVariant=LinkedHashMap<String,String>();t.hashes.forEach{(k,v)->dirVariant[if(k=="lib/Data.js")"LIB/Data.js" else k]=v}
        fails{McpRuntime.verify(t.root,dirVariant,serverOnly=true)}
    }

    @Test fun rootMustBeCanonicalAndManifestNamesMustStayInside()=tree().use{t->
        fails{McpRuntime.verify(t.root.resolve("mcp").resolve(".."),t.hashes,serverOnly=true)}
        for(bad in listOf("../outside","mcp//x","mcp\\x","./x"))fails{McpRuntime.verify(t.root,LinkedHashMap(t.hashes).apply{put(bad,"a".repeat(64))},serverOnly=true)}
    }

    @Test fun directoriesAtTheDepthLimitAreRefused()=tree().use{t->
        Files.createDirectories((1..McpRuntime.MAX_DEPTH).fold(t.root){p,_->p.resolve("d")})
        fails{t.verify()}
    }

    @Test fun symbolicLinksInsideTheTreeAreRefused()=tree().use{t->
        val target=t.root.resolve("mcp/codex/client.mjs");val link=t.root.resolve("mcp/codex/link.mjs")
        val made=runCatching{Files.createSymbolicLink(link,target)}.isSuccess
        assumeTrue("Symbolic link creation is not permitted for this user",made)
        val hashes=LinkedHashMap(t.hashes).apply{put("mcp/codex/link.mjs",t.hashes.getValue("mcp/codex/client.mjs"))}
        fails{McpRuntime.verify(t.root,hashes,serverOnly=true)}
    }

    @Test fun directoryJunctionsInsideTheTreeAreRefused()=tree().use{t->
        assumeTrue("Requires Windows directory junctions",windows)
        val outside=Files.createTempDirectory("vw-mcp-verify-outside-").toRealPath()
        try{
            Files.write(outside.resolve("index.js"),"content:mcp/node_modules/j/index.js".toByteArray())
            val junction=t.root.resolve("mcp/node_modules/j");Files.createDirectories(junction.parent)
            val process=ProcessBuilder("cmd.exe","/c","mklink","/J",junction.toString(),outside.toString()).redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.DISCARD).start()
            assumeTrue("mklink /J did not complete",process.waitFor(20,TimeUnit.SECONDS) && process.exitValue()==0)
            val hashes=LinkedHashMap(t.hashes).apply{put("mcp/node_modules/j/index.js",sha("content:mcp/node_modules/j/index.js".toByteArray()))}
            fails{McpRuntime.verify(t.root,hashes,serverOnly=true)}
            Files.delete(junction) // Removes the junction only, never its target.
            assertTrue(Files.exists(outside.resolve("index.js")))
        }finally{deleteFixture(outside)}
    }

    /** Regression for B8: the earlier per-file toRealPath of every ancestor
     * took about 60 s for the packaged 1,514-file bundle. A deep synthetic tree
     * must stay linear in its entry count. The bound is deliberately loose. */
    @Test fun deepManyFileTreeVerifiesWithoutPerAncestorLookups(){
        val deep=(1..18).joinToString("/"){"n$it"}
        tree((0 until 600).map{"mcp/node_modules/$deep/p${it%20}/f$it.js"}).use{t->
            val started=System.nanoTime()
            t.verify()
            val elapsedMs=(System.nanoTime()-started)/1_000_000
            assertTrue("verify took $elapsedMs ms",elapsedMs<15_000)
        }
    }
}
