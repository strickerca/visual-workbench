import java.security.MessageDigest
import java.io.File
import java.nio.file.Files as NioFiles
import java.nio.file.LinkOption as NioLinkOption
import org.gradle.api.tasks.compile.JavaCompile

plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.compose.multiplatform)
}
kotlin { jvmToolchain(21) }
sourceSets.main { resources.srcDir(rootProject.file("../third_party/notices")) }
dependencies {
    implementation(project(":shared"))
    implementation(project(":bindings-core"))
    implementation(project(":bindings-host"))
    implementation(compose.desktop.currentOs)
    implementation(compose.material)
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.9.0")
    implementation("net.java.dev.jna:jna:5.19.1")
    implementation("com.google.zxing:core:3.5.4")
    testImplementation("junit:junit:4.13.2")
}
compose.desktop {
    application {
        mainClass = "com.visualworkbench.desktop.MainKt"
        jvmArgs += listOf("-Dsun.java2d.dpiaware=true", "-Dskiko.renderApi=DIRECT3D")
        nativeDistributions {
            packageName = "VisualWorkbenchDev"
            packageVersion = "0.0.1"
            modules("java.desktop", "java.logging", "jdk.unsupported")
            // Explicit diagnostic packaging only; ordinary builds keep a GUI launcher.
            windows { console = providers.gradleProperty("vwDesktopSmokeConsole").orNull == "true" }
        }
    }
}
// The shared module generates bindings and packages the prebuilt DLLs. The
// repository build entry point owns the serialized native compilation lane.

// Shared already packages the two DLLs. Package the helper and a complete exact
// inventory from the same native output; no native compilation runs in Gradle.
val runtimeRepository = rootProject.projectDir.parentFile
val runtimeNativeDirectory = providers.gradleProperty("vwNativeDir").map { file(it) }.orElse(runtimeRepository.resolve("target/debug"))
val packageConnectionRuntime = tasks.register("packageConnectionRuntime") {
    val nativeNames = listOf("vw_core.dll", "vw_host.dll", "vw-connection-helper.exe", "vw-capture-helper.exe", "vw-hevc-helper.exe", "vw-input-helper.exe")
    inputs.files(nativeNames.map { name -> runtimeNativeDirectory.map { it.resolve(name) } })
    val output = layout.buildDirectory.dir("generated/connectionRuntimeResources")
    outputs.dir(output)
    doLast {
        val directory = output.get().asFile
        val records = nativeNames.sorted().map { name ->
            val source = runtimeNativeDirectory.get().resolve(name)
            check(source.isFile && source.length() in 1..(256L * 1024 * 1024)) { "Missing or oversized prebuilt $name; use the serialized build.ps1 native lane" }
            val hash = MessageDigest.getInstance("SHA-256")
            source.inputStream().use { input -> val buffer = ByteArray(64 * 1024); while (true) { val n = input.read(buffer); if (n < 0) break; hash.update(buffer, 0, n) } }
            val hex = hash.digest().joinToString("") { "%02x".format(it.toInt() and 255) }
            "$hex ${source.length()} $name"
        }
        directory.mkdirs()
        directory.resolve("win32-x86-64").mkdirs()
        runtimeNativeDirectory.get().resolve("vw-connection-helper.exe").copyTo(directory.resolve("win32-x86-64/vw-connection-helper.exe"), overwrite = true)
        runtimeNativeDirectory.get().resolve("vw-capture-helper.exe").copyTo(directory.resolve("win32-x86-64/vw-capture-helper.exe"), overwrite = true)
        for(name in listOf("vw-hevc-helper.exe","vw-input-helper.exe"))runtimeNativeDirectory.get().resolve(name).copyTo(directory.resolve("win32-x86-64/$name"),overwrite=true)
        directory.resolve("vw-native-runtime.sha256").writeText(records.joinToString("\n", postfix = "\n"), Charsets.US_ASCII)
    }
}
sourceSets.named("main") { resources.srcDir(packageConnectionRuntime) }

// Optional exact root-admitted editor catalog. This is a build-owner asset, never
// a runtime file override. Native current evidence still decides every action.
val remoteEditorCatalog=providers.gradleProperty("vwRemoteEditorCatalog").map{file(it)}
val remoteEditorCatalogSha256=providers.gradleProperty("vwRemoteEditorCatalogSha256")
val packageRemoteEditorCatalog=tasks.register("packageRemoteEditorCatalog") {
    inputs.property("enabled",remoteEditorCatalog.isPresent)
    inputs.property("sha256",remoteEditorCatalogSha256.orElse(""))
    if(remoteEditorCatalog.isPresent)inputs.file(remoteEditorCatalog)
    val output=layout.buildDirectory.dir("generated/remoteEditorCatalogResources")
    outputs.dir(output)
    doLast {
        check(remoteEditorCatalog.isPresent==remoteEditorCatalogSha256.isPresent){"Editor catalog path and admitted SHA256 must be supplied together"}
        val directory=output.get().asFile.absoluteFile.normalize()
        check(directory.toPath().startsWith(layout.buildDirectory.get().asFile.absoluteFile.normalize().toPath())){"Editor catalog output escaped the task build directory"}
        for(part in generateSequence(directory){it.parentFile}.toList().asReversed()) {
            val path=part.toPath()
            if(NioFiles.exists(path,NioLinkOption.NOFOLLOW_LINKS)) {
                check(NioFiles.isDirectory(path,NioLinkOption.NOFOLLOW_LINKS)&&part.canonicalFile==part.absoluteFile.normalize()){"Editor catalog output namespace is redirected"}
            } else {
                check(part.mkdir()){"Editor catalog output directory could not be created"}
                check(part.canonicalFile==part.absoluteFile.normalize()){"Editor catalog output namespace changed"}
            }
        }
        val names=setOf("vw-remote-editor-catalog.json","vw-remote-editor-catalog.sha256")
        check(directory.listFiles()?.all{it.isFile&&it.name in names&&it.canonicalFile==it.absoluteFile.normalize()}==true){"Unexpected generated editor catalog entries"}
        if(!remoteEditorCatalog.isPresent) {
            for(name in names){val stale=directory.resolve(name);check(!stale.exists()||stale.delete()){"Could not remove stale generated editor catalog"}}
        } else {
            val expected=remoteEditorCatalogSha256.get();check(expected.matches(Regex("[a-f0-9]{64}"))){"Invalid admitted editor catalog SHA256"}
            val source=remoteEditorCatalog.get()
            check(source.isFile&&source.canonicalFile==source.absoluteFile.normalize()&&source.length() in 1..(32L*1024)){"Editor catalog must be a canonical bounded admitted file"}
            val bytes=source.inputStream().use{it.readNBytes(32*1024+1)}
            check(bytes.size in 1..(32*1024)){"Editor catalog changed beyond its bound"}
            val actual=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}
            check(actual==expected){"Editor catalog differs from its root-admitted SHA256"}
            directory.resolve("vw-remote-editor-catalog.json").writeBytes(bytes)
            directory.resolve("vw-remote-editor-catalog.sha256").writeText("$actual ${bytes.size} vw-remote-editor-catalog.json\n",Charsets.US_ASCII)
        }
    }
}
sourceSets.named("main"){resources.srcDir(packageRemoteEditorCatalog)}

// A successful central package build must contain this exact fresh receipt ID.
// No build ID is emitted for ordinary desktop builds.
val desktopBuildId = providers.gradleProperty("vwDesktopBuildId")
val packageDesktopBuildReceipt = tasks.register("packageDesktopBuildReceipt") {
    inputs.property("receiptId", desktopBuildId.orElse(""))
    val output = layout.buildDirectory.dir("generated/desktopBuildReceipt")
    outputs.dir(output)
    doLast {
        val id = desktopBuildId.orNull
        check(id == null || id.matches(Regex("[0-9a-f]{32}"))) { "Invalid desktop build receipt" }
        val directory = output.get().asFile
        directory.mkdirs()
        val marker = directory.resolve("vw-desktop-build.id")
        if (id == null) {
            check(!marker.exists() || marker.delete()) { "Could not remove old desktop build receipt" }
        } else marker.writeText(id + "\n", Charsets.US_ASCII)
    }
}
sourceSets.named("main") { resources.srcDir(packageDesktopBuildReceipt) }

// MCP server resources are produced offline from a bounded staged production
// runtime BEFORE the application JAR; the final bridge is built AFTER jpackage.
// This property is an explicit build-owner input, never a runtime override.
val mcpServerResources = providers.gradleProperty("vwMcpServerResources").map { file(it) }
val admitMcpServerResources = tasks.register("admitMcpServerResources") {
    inputs.property("enabled", mcpServerResources.isPresent)
    if (mcpServerResources.isPresent) inputs.dir(mcpServerResources)
    doLast {
        if (mcpServerResources.isPresent) {
            val folder = mcpServerResources.get()
            check(folder.isDirectory && folder.canonicalFile == folder.absoluteFile.normalize()) { "MCP resources require a canonical private generated directory" }
            val manifest = folder.resolve("vw-mcp-server.sha256")
            check(manifest.isFile && manifest.length() in 1..(4L * 1024 * 1024)) { "Missing generated MCP server inventory" }
            check(folder.resolve("mcp-server").isDirectory) { "Missing generated MCP server payload" }
            // Runtime admission verifies every byte and rejects unlisted files.
            // Packaging must not add arbitrary classpath resources from this path.
            check(folder.listFiles()!!.map { it.name }.toSet() == setOf("mcp-server", "vw-mcp-server.sha256")) { "Unexpected MCP resource roots" }
        }
    }
}
if (mcpServerResources.isPresent) sourceSets.named("main") { resources.srcDir(mcpServerResources) }
tasks.named("processResources") { dependsOn(admitMcpServerResources) }

// Root serializes compilation first. The finite runner reads this fresh classpath
// receipt and starts Java directly in an owned Job; it never builds while live.
val integrationMainJava = tasks.named<JavaCompile>(sourceSets.main.get().compileJavaTaskName)
tasks.register("remoteIntegrationClasspath") {
    dependsOn(tasks.named("classes"))
    val destination=layout.buildDirectory.file("remote-integration-classpath.txt")
    outputs.file(destination)
    outputs.upToDateWhen { false } // Re-evaluate this cheap receipt against actual current task/artifact state.
    doLast {
        check(providers.gradleProperty("vwRemoteIntegration").orNull=="true")
        val javaCompile = integrationMainJava.get()
        val noJavaOutput = javaCompile.state.executed && javaCompile.state.noSource && javaCompile.source.isEmpty
        val declaredJavaOutput = javaCompile.destinationDirectory.get().asFile.absoluteFile.normalize()
        val entries = sourceSets.main.get().runtimeClasspath.files.filter { entry ->
            // Only this exact declared output may be omitted after actual
            // NO-SOURCE execution. Every other missing artifact still fails.
            if (noJavaOutput && entry.absoluteFile.normalize() == declaredJavaOutput) false
            else {
                check(entry.exists()) { "Missing integration classpath entry: $entry" }
                true
            }
        }
        check(entries.isNotEmpty()) { "Empty integration classpath" }
        destination.get().asFile.writeText(entries.joinToString(File.pathSeparator) { it.path },Charsets.UTF_8)
    }
}
