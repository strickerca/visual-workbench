package com.visualworkbench.shared

internal actual fun prepareCoreLibrary() { System.loadLibrary("vw_core"); AndroidOsImages.install(); AndroidAvifImages.install(); com.visualworkbench.shared.giant.AndroidGiantRuntime.install() }
