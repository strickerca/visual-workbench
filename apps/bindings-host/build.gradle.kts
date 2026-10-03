import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins { alias(libs.plugins.kotlin.jvm) }

// Desktop alone consumes this generated module. Keep the exact generator
// output unchanged and handwritten shared sources under strict explicitApi().
val generatedBindings = project(":shared").layout.buildDirectory.dir("generated/uniffi/host")
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
    compileOnly("net.java.dev.jna:jna:5.19.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
}
gradle.projectsEvaluated {
    // Use the resolved task provider before the execution-time license census.
    val generator = project(":shared").tasks.named("generateHostBindings")
    tasks.named("compileKotlin") { dependsOn(generator) }
}
