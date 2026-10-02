plugins { alias(libs.plugins.android.application) }

android {
    namespace = "com.visualworkbench.videobench"
    compileSdk = 37
    buildToolsVersion = "36.1.0"
    defaultConfig {
        applicationId = "com.visualworkbench.videobench"
        minSdk = 30
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-t007"
    }
}
