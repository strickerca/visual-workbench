plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.kmp.library)
}
kotlin {
    explicitApi()
    jvmToolchain(21)
    android {
        namespace = "com.visualworkbench.shared"
        compileSdk = 37
        minSdk = 29
    }
    jvm("desktop")
}
