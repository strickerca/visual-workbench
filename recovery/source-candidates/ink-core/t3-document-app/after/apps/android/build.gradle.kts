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
        manifestPlaceholders["vwDocumentExtractNative"] = providers.gradleProperty("vwDocumentAndroidResources").isPresent.toString()
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
        jniLibs.useLegacyPackaging = providers.gradleProperty("vwDocumentAndroidResources").isPresent
        // Preserve the independently built core bytes for the APK binding check.
        jniLibs.keepDebugSymbols += "**/libvw_core.so"
    }
    sourceSets.getByName("main").assets.srcDir(rootProject.file("../third_party/notices"))
}
dependencies {
    implementation(project(":shared"))
    implementation(project(":bindings-core"))
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

// Root alone supplies an independently admitted arm64 PDFium build. Its source,
// binary hash, notices and 16 KiB PT_LOAD acceptance are mandatory. No download.
val documentAndroidResources=providers.gradleProperty("vwDocumentAndroidResources").map{file(it)}
val admitAndroidDocumentRuntime=tasks.register("admitAndroidDocumentRuntime") {
    inputs.property("enabled",documentAndroidResources.isPresent)
    if(documentAndroidResources.isPresent)inputs.dir(documentAndroidResources)
    doLast{if(documentAndroidResources.isPresent){
        val folder=documentAndroidResources.get();check(folder.isDirectory&&folder.canonicalFile==folder.absoluteFile.normalize())
        check(folder.listFiles()!!.map{it.name}.toSet()==setOf("assets","jni","vw-document-admission.properties"))
        val admission=java.util.Properties().apply{folder.resolve("vw-document-admission.properties").inputStream().use{load(it)}}
        check(admission.getProperty("admitted-by-build-owner")=="true"&&admission.getProperty("pdfium-api")=="7881"&&admission.getProperty("licenses-reviewed")=="true"&&admission.getProperty("lgpl-artifacts")=="false"&&admission.getProperty("elf-16k-reviewed")=="true"&&admission.getProperty("extracted-apk-checker-reviewed")=="true")
        check(admission.getProperty("pdfium-source-sha256")?.matches(Regex("[0-9a-f]{64}"))==true)
        val inventory=folder.resolve("assets/vw-document-android.sha256");check(inventory.isFile&&inventory.length() in 1..1024)
        val match=Regex("([0-9a-f]{64}) ([1-9][0-9]{0,8}) libpdfium\\.so\\n?").matchEntire(inventory.readText(Charsets.US_ASCII))?:error("Invalid admitted PDFium inventory")
        check(folder.resolve("jni").listFiles()!!.map{it.name}.toSet()==setOf("arm64-v8a"));check(folder.resolve("jni/arm64-v8a").listFiles()!!.map{it.name}.toSet()==setOf("libpdfium.so"))
        val source=folder.resolve("jni/arm64-v8a/libpdfium.so");check(source.isFile&&source.canonicalFile==source.absoluteFile.normalize()&&source.length()==match.groupValues[2].toLong()&&source.length()<=128L*1024*1024)
        val digest=java.security.MessageDigest.getInstance("SHA-256");source.inputStream().use{input->val buffer=ByteArray(65536);while(true){val count=input.read(buffer);if(count<0)break;digest.update(buffer,0,count)}};check(digest.digest().joinToString(""){"%02x".format(it.toInt() and 255)}==match.groupValues[1])
    }}
}
if(documentAndroidResources.isPresent){android.sourceSets.getByName("main").apply{jniLibs.srcDir(documentAndroidResources.map{it.resolve("jni")});assets.srcDir(documentAndroidResources.map{it.resolve("assets")})}}
tasks.matching{it.name.startsWith("merge")&&(it.name.endsWith("NativeLibs")||it.name.endsWith("Assets"))}.configureEach{dependsOn(admitAndroidDocumentRuntime)}
