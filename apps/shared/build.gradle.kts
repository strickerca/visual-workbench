import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.attribute.BasicFileAttributes

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.kmp.library)
}
val repository = rootProject.projectDir.parentFile
val nativeDirectory = providers.gradleProperty("vwNativeDir").map { file(it) }.orElse(repository.resolve("target/debug"))
val androidNativeDirectory = providers.gradleProperty("vwAndroidNativeDir").map { file(it) }.orElse(repository.resolve("target/android-jni"))
fun generatedBindingsDirectory(component: String): java.io.File {
    check(component in setOf("core", "host"))
    val output = layout.buildDirectory.dir("generated/uniffi/$component").get().asFile
    val expected = file("build/generated/uniffi/$component").absoluteFile.normalize()
    // Only this task's generated directory may be removed, never a redirected
    // path or another build tree. This prevents stale generated packages.
    check(output.absoluteFile.normalize() == expected && output.canonicalFile == expected) {
        "Unexpected or redirected UniFFI output directory"
    }
    return output
}
fun checkGeneratedBindingsTree(output: java.io.File) {
    val root = output.toPath().toAbsolutePath().normalize()
    if (!Files.exists(root, LinkOption.NOFOLLOW_LINKS)) return
    // Inspect a bounded, non-following tree before deleting or reading it. The
    // pinned generator currently produces four package directories and one file.
    var entries = 0
    var bytes = 0L
    Files.walk(root).use { paths ->
        val iterator = paths.iterator()
        while (iterator.hasNext()) {
            val path = iterator.next()
            check(++entries <= 64 && path.startsWith(root) && root.relativize(path).nameCount <= 8) {
                "UniFFI output tree exceeds its reviewed bounds"
            }
            val attributes = Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS)
            check(!attributes.isSymbolicLink && !attributes.isOther &&
                (attributes.isDirectory || attributes.isRegularFile) && path.toRealPath() == path) {
                "Redirected or unsupported entry in UniFFI output tree"
            }
            if (attributes.isRegularFile) {
                bytes = Math.addExact(bytes, attributes.size())
                check(bytes <= 16L * 1024 * 1024) { "UniFFI generated sources exceed the size bound" }
            }
        }
    }
}
fun cleanGeneratedBindings(component: String) {
    val output = generatedBindingsDirectory(component)
    checkGeneratedBindingsTree(output)
    delete(output)
}
fun assertGeneratedBindings(component: String) {
    val output = generatedBindingsDirectory(component)
    checkGeneratedBindingsTree(output)
    val expected = output.resolve("com/visualworkbench/bindings/$component/vw_$component.kt").absoluteFile.normalize()
    val files = fileTree(output).files
    check(files.size == 1 && files.single().absoluteFile.normalize() == expected && expected.canonicalFile == expected) {
        "UniFFI generated an unexpected Kotlin filename or additional files"
    }
    val generated = files.single()
    val source = generated.readText(Charsets.UTF_8)
    val packages = source.lineSequence().filter { it.startsWith("package ") }.toList()
    check(packages == listOf("package com.visualworkbench.bindings.$component")) { "UniFFI ignored the configured Kotlin package" }
    val original = "@file:Suppress(\"NAME_SHADOWING\")"
    check(source.split(original).size == 2) { "UniFFI generated header changed; review the pinned generator" }
    // Generated source remains byte-for-byte upstream output. It compiles in a
    // separate JVM module; every handwritten shared source retains explicitApi.
}
val generateCoreBindings = tasks.register<Exec>("generateCoreBindings") {
    val library = nativeDirectory.map { it.resolve("vw_core.dll") }
    val generator = nativeDirectory.map { it.resolve("vw-bindgen.exe") }
    inputs.file(library); inputs.file(generator); inputs.file(repository.resolve("core/crates/vw-ffi/uniffi.toml"))
    inputs.property("generatedJvmBoundary", 1)
    outputs.dir(layout.buildDirectory.dir("generated/uniffi/core"))
    doFirst {
        cleanGeneratedBindings("core")
        commandLine(generator.get().absolutePath,"generate","--library",library.get().absolutePath,"--language","kotlin","--config",repository.resolve("core/crates/vw-ffi/uniffi.toml").absolutePath,"--out-dir",layout.buildDirectory.dir("generated/uniffi/core").get().asFile.absolutePath,"--no-format")
    }
    doLast { assertGeneratedBindings("core") }
}
val generateHostBindings = tasks.register<Exec>("generateHostBindings") {
    val library = nativeDirectory.map { it.resolve("vw_host.dll") }
    inputs.file(library); inputs.file(nativeDirectory.map { it.resolve("vw-bindgen.exe") }); inputs.file(repository.resolve("host-win/crates/vw-host-ffi/uniffi.toml"))
    inputs.property("generatedJvmBoundary", 1)
    outputs.dir(layout.buildDirectory.dir("generated/uniffi/host"))
    doFirst {
        cleanGeneratedBindings("host")
        commandLine(nativeDirectory.get().resolve("vw-bindgen.exe").absolutePath,"generate","--library",library.get().absolutePath,"--language","kotlin","--config",repository.resolve("host-win/crates/vw-host-ffi/uniffi.toml").absolutePath,"--out-dir",layout.buildDirectory.dir("generated/uniffi/host").get().asFile.absolutePath,"--no-format")
    }
    doLast { assertGeneratedBindings("host") }
}
val packageDesktopNative = tasks.register<Sync>("packageDesktopNative") {
    from(nativeDirectory) { include("vw_core.dll","vw_host.dll"); into("win32-x86-64") }
    into(layout.buildDirectory.dir("generated/nativeResources"))
    doFirst { for (name in listOf("vw_core.dll","vw_host.dll")) check(nativeDirectory.get().resolve(name).isFile) { "Missing prebuilt $name; use build.ps1" } }
}
val checkCommonMainImports = tasks.register<Exec>("checkCommonMainImports") {
    commandLine("python",repository.resolve("tools/ffi-test/check_imports.py").absolutePath,file("src/commonMain").absolutePath)
    inputs.dir("src/commonMain"); inputs.file(repository.resolve("tools/ffi-test/check_imports.py"))
}
val checkImportGuardFixtures = tasks.register<Exec>("checkImportGuardFixtures") {
    workingDir(repository.resolve("tools/ffi-test"))
    commandLine("python","-m","unittest","test_imports")
}
val checkAndroidNativeAlignment = tasks.register<Exec>("checkAndroidNativeAlignment") {
    val sdk = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
    doFirst {
        check(sdk != null) { "ANDROID_HOME required for the pinned NDK alignment check" }
        commandLine("powershell","-NoProfile","-ExecutionPolicy","Bypass","-File",repository.resolve("tools/ffi-test/check-alignment.ps1").absolutePath,"-Library",androidNativeDirectory.get().resolve("arm64-v8a/libvw_core.so").absolutePath,"-Objdump",file("$sdk/ndk/30.0.16248370/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-objdump.exe").absolutePath)
    }
}
kotlin {
    explicitApi()
    jvmToolchain(21)
    android {
        namespace = "com.visualworkbench.shared"
        compileSdk = 37
        minSdk = 29
        packaging {
            jniLibs {
                useLegacyPackaging = false
                // This app supports arm64 only. KMP device tests do not inherit
                // the app module's ndk.abiFilters; exclude dependency ABIs here.
                excludes += setOf("**/armeabi/**", "**/armeabi-v7a/**", "**/x86/**",
                    "**/x86_64/**", "**/mips/**", "**/mips64/**", "**/riscv64/**")
                // Bind the final APK to the exact independently built core.
                keepDebugSymbols += "**/libvw_core.so"
            }
        }
        withDeviceTest { instrumentationRunner = "androidx.test.runner.AndroidJUnitRunner" }
    }
    jvm("desktop")
    sourceSets {
        commonMain.dependencies { implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0") }
        commonTest.dependencies { implementation(kotlin("test")) }
        androidMain {
            kotlin.srcDir("src/jvmMain/kotlin")
            dependencies {
                implementation(project(":bindings-core"))
                implementation("net.java.dev.jna:jna:5.19.1@aar")
            }
        }
        val desktopMain by getting {
            kotlin.srcDir("src/jvmMain/kotlin")
            resources.srcDir(packageDesktopNative)
            dependencies {
                implementation(project(":bindings-core"))
                implementation("net.java.dev.jna:jna:5.19.1")
            }
        }
        val desktopTest by getting {
            kotlin.srcDir("src/jvmTest/kotlin")
            resources.srcDir(repository.resolve("tools/ffi-test/fixtures"))
            dependencies { implementation(kotlin("test-junit")); implementation("junit:junit:4.13.2") }
        }
        getByName("androidDeviceTest") {
            kotlin.srcDir("src/jvmTest/kotlin")
            resources.srcDir(repository.resolve("tools/ffi-test/fixtures"))
            dependencies {
                implementation("androidx.test:runner:1.7.0")
                implementation("androidx.test:core:1.7.0")
                implementation("androidx.test.ext:junit:1.3.0")
                implementation("junit:junit:4.13.2")
            }
        }
    }
}
androidComponents { onVariants { variant -> variant.sources.jniLibs?.addStaticSourceDirectory(androidNativeDirectory.get().absolutePath) } }
tasks.matching { it.name.startsWith("compile") && it.name.contains("Kotlin") }.configureEach { dependsOn(checkCommonMainImports) }
tasks.matching { it.name.contains("Android") && it.name.startsWith("compile") }.configureEach { dependsOn(checkAndroidNativeAlignment) }
tasks.named("check") { dependsOn(checkCommonMainImports,checkImportGuardFixtures) }
