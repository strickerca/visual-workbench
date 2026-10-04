pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositories {
        google(); mavenCentral()
        exclusiveContent {
            forRepository {
                maven { url = uri("https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/") }
            }
            filter { includeGroup("org.rustls") }
        }
    }
}
rootProject.name = "visual-workbench-apps"
include(":shared", ":android", ":desktop")
include(":bindings-core", ":bindings-host")
include(":pen-probe")
project(":pen-probe").projectDir = file("../tools/pen-trace/probe-android")
include(":video-bench")
project(":video-bench").projectDir = file("../tools/bench/video-android")
include(":image-bench")
project(":image-bench").projectDir = file("../tools/bench/image-android")
include(":stroke-spike")
project(":stroke-spike").projectDir = file("../tools/stroke-spike/android")
