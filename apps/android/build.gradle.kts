import org.gradle.api.file.DirectoryProperty
import org.gradle.api.tasks.OutputDirectory

abstract class RustAndroidBuild : Exec() {
    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty
}

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}
android {
    namespace = "com.visualworkbench.android"
    compileSdk = 37
    buildToolsVersion = "36.1.0"
    ndkVersion = "30.0.16248370"
    defaultConfig {
        applicationId = "com.visualworkbench.android"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.0.1-dev"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "arm64-v8a" }
    }
    buildFeatures { compose = true }
    packaging { jniLibs.useLegacyPackaging = false }
}
dependencies {
    implementation(project(":shared"))
    implementation(platform(libs.compose.bom))
    implementation(libs.activity.compose)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material:material")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
val buildRustAndroid = tasks.register<RustAndroidBuild>("buildRustAndroid") {
    outputDirectory.set(layout.buildDirectory.dir("generated/rustJniLibs"))
    workingDir(rootProject.projectDir.parentFile)
    val sdk = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
    if (sdk != null) environment("ANDROID_NDK_HOME", "$sdk/ndk/30.0.16248370")
    commandLine("cargo", "+1.99.0", "ndk", "-t", "arm64-v8a", "--platform", "29", "-o",
        outputDirectory.get().asFile.absolutePath,
        "build", "-p", "vw-ffi", "--locked")
}
tasks.named("preBuild") { dependsOn(buildRustAndroid) }
androidComponents {
    onVariants(selector().all()) { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(buildRustAndroid, RustAndroidBuild::outputDirectory)
    }
}
