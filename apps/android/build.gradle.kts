plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}
// Opt-in isolated HIL uses the same debug source/dependency graph. A typo must
// never select or overwrite normal app artifacts.
val vwAndroidHilValue = providers.gradleProperty("vwAndroidHil").orNull
check(vwAndroidHilValue == null || vwAndroidHilValue == "true") { "vwAndroidHil must be absent or exactly true" }
val vwAndroidHil = vwAndroidHilValue == "true"
if (vwAndroidHil) layout.buildDirectory.set(layout.projectDirectory.dir("build-hil"))
android {
    namespace = "com.visualworkbench.android"
    compileSdk = 37
    buildToolsVersion = "36.1.0"
    ndkVersion = "30.0.16248370"
    defaultConfig {
        applicationId = "com.visualworkbench.android"
        if (vwAndroidHil) applicationIdSuffix = ".hil"
        manifestPlaceholders["vwApplicationLabel"] = if (vwAndroidHil) "Visual Workbench Test" else "Visual Workbench"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.0.1-dev"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "arm64-v8a" }
    }
    buildFeatures { compose = true }
    packaging {
        jniLibs.useLegacyPackaging = false
        // Preserve the independently built core bytes for the APK binding check.
        jniLibs.keepDebugSymbols += "**/libvw_core.so"
    }
    sourceSets.getByName("main").assets.srcDir(rootProject.file("../third_party/notices"))
}
dependencies {
    implementation(project(":shared"))
    implementation(platform(libs.compose.bom))
    implementation(libs.activity.compose)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material:material")
    implementation("androidx.graphics:graphics-core:1.0.4")
    implementation("androidx.input:input-motionprediction:1.0.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.9.4")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("androidx.camera:camera-core:1.6.2")
    implementation("androidx.camera:camera-camera2:1.6.2")
    implementation("androidx.camera:camera-lifecycle:1.6.2")
    implementation("androidx.camera:camera-view:1.6.2")
    implementation("com.google.zxing:core:3.5.4")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
// The shared module packages the prebuilt core and checks ELF alignment.
// build.ps1 owns the serialized Rust build; do not compile/package it twice.
