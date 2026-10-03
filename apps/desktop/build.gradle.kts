import java.security.MessageDigest

plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.compose.multiplatform)
}
kotlin { jvmToolchain(21) }
sourceSets.main { resources.srcDir(rootProject.file("../third_party/notices")) }
dependencies {
    implementation(project(":shared"))
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
    val nativeNames = listOf("vw_core.dll", "vw_host.dll", "vw-connection-helper.exe")
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
        directory.resolve("vw-native-runtime.sha256").writeText(records.joinToString("\n", postfix = "\n"), Charsets.US_ASCII)
    }
}
sourceSets.named("main") { resources.srcDir(packageConnectionRuntime) }

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
