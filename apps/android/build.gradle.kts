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
val remoteIntegrationValue=providers.gradleProperty("vwRemoteIntegration").orNull
check(remoteIntegrationValue==null||remoteIntegrationValue=="true")
val remoteIntegration=remoteIntegrationValue=="true"
check(!remoteIntegration||vwAndroidHil) { "Real remote integration requires the isolated HIL application" }
if(remoteIntegration)layout.buildDirectory.set(layout.projectDirectory.dir("build-remote-integration"))
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
    buildTypes {
        getByName("release") {
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    packaging {
        jniLibs.useLegacyPackaging = false
        // Preserve the independently built core bytes for the APK binding check.
        jniLibs.keepDebugSymbols += "**/libvw_core.so"
    }
    sourceSets.getByName("main").assets.srcDir(rootProject.file("../third_party/notices"))
    if(remoteIntegration) {
        // This opt-in source graph has its own real integration census. The
        // ordinary166-case app HIL source set and artifacts stay unchanged.
        sourceSets.getByName("debug").kotlin.directories.add("src/remoteIntegration/kotlin")
        sourceSets.getByName("debug").manifest.srcFile("src/remoteIntegration/AndroidManifest.xml")
        // Built-in Kotlin has its own source directory set. Replace both test
        // sets explicitly only in this opt-in variant for the exact one-case census.
        sourceSets.getByName("androidTest").java.directories.clear()
        sourceSets.getByName("androidTest").kotlin.directories.apply {
            clear()
            add("src/remoteIntegrationTest/kotlin")
        }
    }
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
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
// The shared module packages the prebuilt core and checks ELF alignment.
// build.ps1 owns the serialized Rust build; do not compile/package it twice.
