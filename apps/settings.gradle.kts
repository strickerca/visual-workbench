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
