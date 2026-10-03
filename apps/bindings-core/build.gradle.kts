import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins { alias(libs.plugins.kotlin.jvm) }

// Only upstream-generated declarations compile here. The public handwritten
// shared facade keeps explicitApi(); no generated diagnostics are suppressed.
val generatedBindings = project(":shared").layout.buildDirectory.dir("generated/uniffi/core")
kotlin {
    jvmToolchain(21)
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
    sourceSets.named("main") { kotlin.setSrcDirs(listOf(generatedBindings)) }
}
java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}
sourceSets.named("main") {
    java.setSrcDirs(emptyList<String>())
    resources.setSrcDirs(emptyList<String>())
}
dependencies {
    // The Android consumer supplies JNA's AAR; publishing its jar transitively
    // would add a second copy of JNA's classes to the APK.
    compileOnly("net.java.dev.jna:jna:5.19.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
}
gradle.projectsEvaluated {
    // Resolve the provider after both projects are configured. A string task
    // path resolved during the license task's producer census re-enters Gradle
    // project configuration from an execution worker and deadlocks on 9.7.0.
    val generator = project(":shared").tasks.named("generateCoreBindings")
    tasks.named("compileKotlin") { dependsOn(generator) }
}
