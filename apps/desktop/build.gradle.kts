plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.compose.multiplatform)
}
kotlin { jvmToolchain(21) }
dependencies {
    implementation(project(":shared"))
    implementation(compose.desktop.currentOs)
    implementation(compose.material)
}
compose.desktop {
    application {
        mainClass = "com.visualworkbench.desktop.MainKt"
        jvmArgs += listOf("-Dsun.java2d.dpiaware=true", "-Dskiko.renderApi=DIRECT3D")
        nativeDistributions {
            packageName = "VisualWorkbenchDev"
            packageVersion = "0.0.1"
            modules("java.desktop", "java.logging", "jdk.unsupported")
        }
    }
}
val buildRustWindows = tasks.register<Exec>("buildRustWindows") {
    workingDir(rootProject.projectDir.parentFile)
    commandLine("cargo", "+1.99.0", "build", "--target", "x86_64-pc-windows-msvc", "-p", "vw-ffi", "-p", "vw-host-ffi", "--locked")
}
val copyRustWindows = tasks.register<Copy>("copyRustWindows") {
    dependsOn(buildRustWindows)
    from(rootProject.projectDir.parentFile.resolve("target/x86_64-pc-windows-msvc/debug")) {
        include("vw_core.dll", "vw_host.dll")
    }
    into(layout.buildDirectory.dir("generated/rustResources/native"))
}
sourceSets.main { resources.srcDir(layout.buildDirectory.dir("generated/rustResources")) }
tasks.named("processResources") { dependsOn(copyRustWindows) }
