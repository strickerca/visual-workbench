plugins { alias(libs.plugins.android.application) }

android {
    namespace = "com.visualworkbench.imagebench"
    compileSdk = 37
    buildToolsVersion = "36.1.0"
    defaultConfig {
        applicationId = "com.visualworkbench.imagebench"
        minSdk = 30
        targetSdk = 36
        versionCode = 3
        versionName = "0.1.0-t009c"
    }
}

dependencies { implementation("androidx.heifwriter:heifwriter:1.1.0") }
