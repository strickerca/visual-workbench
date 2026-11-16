pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositories {
        google(); mavenCentral()
        exclusiveContent {
            forRepository {
                // VW_RUSTLS_MAVEN points at a local checkout (file: URL only) of the same branch where the
                // network cannot reach this URL (cloud sessions). The lock state still pins
                // the exact version either way.
                maven { url = uri(providers.environmentVariable("VW_RUSTLS_MAVEN").orNull
                    ?.also { require(it.startsWith("file:")) { "VW_RUSTLS_MAVEN must be a file: URL" } }
                    ?: "https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/") }
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
