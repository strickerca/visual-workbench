pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositories { google(); mavenCentral() }
}
rootProject.name = "visual-workbench-apps"
include(":shared", ":android", ":desktop")
include(":pen-probe")
project(":pen-probe").projectDir = file("../tools/pen-trace/probe-android")
include(":video-bench")
project(":video-bench").projectDir = file("../tools/bench/video-android")
include(":image-bench")
project(":image-bench").projectDir = file("../tools/bench/image-android")
include(":stroke-spike")
project(":stroke-spike").projectDir = file("../tools/stroke-spike/android")
