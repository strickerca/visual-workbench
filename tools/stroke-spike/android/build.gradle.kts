plugins { alias(libs.plugins.android.application) }

android {
    namespace = "com.visualworkbench.strokespike"
    compileSdk = 37
    buildToolsVersion = "36.1.0"
    defaultConfig {
        applicationId = "com.visualworkbench.strokespike"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-t010"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += "arm64-v8a" }
    }
    sourceSets.getByName("main").jniLibs.directories.add("build/generated/jniLibs")
}

dependencies {
    implementation("androidx.graphics:graphics-core:1.0.4")
    implementation("androidx.input:input-motionprediction:1.0.0")
    implementation("androidx.ink:ink-authoring:1.0.0")
    implementation("androidx.ink:ink-brush:1.0.0")
    implementation("androidx.ink:ink-strokes:1.0.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
