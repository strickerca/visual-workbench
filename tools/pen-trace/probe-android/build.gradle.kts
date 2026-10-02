plugins { alias(libs.plugins.android.application) }

android {
    namespace = "com.visualworkbench.penprobe"
    compileSdk = 37
    buildToolsVersion = "36.1.0"
    defaultConfig {
        applicationId = "com.visualworkbench.penprobe"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-t004"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }
    sourceSets.getByName("androidTest").assets.directories.add("../../../fixtures/traces")
}
dependencies {
    implementation("androidx.graphics:graphics-core:1.0.4")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
