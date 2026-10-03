# Android dependency provenance — 2026-10-02

This is a source/artifact inspection note for the T1.10 candidate. No Gradle
resolution, build, installation, device test, or version change was performed.
The exact public Maven URLs below were fetched over HTTPS into memory, hashed,
and inspected as ZIP/XML; no downloaded artifact was installed or retained.
These bytes must still be compared with the central build's resolved inputs.

## Licenses and native contents

| Pin | Inspected evidence | Required treatment |
| --- | --- | --- |
| `androidx.camera:{camera-core,camera-camera2,camera-lifecycle,camera-view}:1.6.2` | Exact four POMs. `camera-core` declares Apache-2.0 **and** BSD-3-Clause; the other three declare Apache-2.0. The core AAR contains `libimage_processing_util_jni.so` and `libsurface_util_jni.so` for four ABIs. | Retain both core license families. AndroidX's [libyuv packaging source](https://android.googlesource.com/platform/frameworks/support/+/refs/heads/androidx-input-release/external/libyuv) explains its static inclusion in camera-core. [Upstream libyuv](https://chromium.googlesource.com/libyuv/libyuv/+/refs/heads/main/README.chromium) identifies BSD-3-Clause. The exact libyuv source revision used for this AAR has **not** been established. |
| `com.google.zxing:core:3.5.4` | Exact child/parent POMs and JAR. License inherited from parent: Apache-2.0. No `.so` entries in this JAR. | Use the [tagged upstream license](https://github.com/zxing/zxing/blob/zxing-3.5.4/LICENSE). The repository-wide license also describes jai-imageio; core's inspected POM has no runtime dependencies. Do not infer that the whole ZXing repository is packaged here. |
| `net.java.dev.jna:jna:5.19.1@aar` (shared Android facade) | Exact POM and AAR. POM defaults to JAR; the explicit `@aar` selects the inspected Android artifact. AAR contains `libjnidispatch.so` for seven ABIs. Nested `classes.jar` contains `META-INF/{AL2.0,LGPL2.1,LICENSE}`. | The [5.19.1 license](https://github.com/java-native-access/jna/blob/5.19.1/LICENSE) permits Apache-2.0 **or** LGPL-2.1-or-later; use the Apache option in the central license inventory. Also retain bundled native libffi's [tagged permissive license](https://github.com/java-native-access/jna/blob/5.19.1/native/libffi/LICENSE). |

CameraX core's embedded `META-INF/androidx/camera/camera-core/LICENSE.txt` is
10,175 bytes, SHA-256
`809fa1ed21450f59827d1e9aec720bbc4b687434fa22283c6cb5dd82a47ab9c0`.
It contains Apache text and no libyuv/BSD notice. The POM's additional BSD
declaration must not disappear merely because an AAR notice scanner sees only
that file. The inspected [libyuv license](https://chromium.googlesource.com/libyuv/libyuv/+/refs/heads/main/LICENSE)
requires retaining its copyright/conditions/disclaimer in binary distribution
materials. Its moving `main` URL is supporting provenance, not an assertion of
the exact bundled source revision.

The raw [JNA-tagged libffi license](https://raw.githubusercontent.com/java-native-access/jna/5.19.1/native/libffi/LICENSE)
was 1,132 bytes, SHA-256
`2c9c2acb9743e6b007b91350475308aee44691d96aa20eacef8e199988c8c388`.
The [tagged native Makefile](https://github.com/java-native-access/jna/blob/5.19.1/native/Makefile)
references the bundled `native/libffi` static library by default. Exact native
reproducibility and all platform-specific notices remain central review work.

## Exact inspected Maven bytes

| Artifact (direct source URL) | Bytes | SHA-256 |
| --- | ---: | --- |
| [camera-core POM](https://dl.google.com/dl/android/maven2/androidx/camera/camera-core/1.6.2/camera-core-1.6.2.pom) | 7,109 | `cddb5bb0653654d8b7c2833484f4684d881dc7509411132e73a5fb4b167fd13c` |
| [camera-camera2 POM](https://dl.google.com/dl/android/maven2/androidx/camera/camera-camera2/1.6.2/camera-camera2-1.6.2.pom) | 6,448 | `23020a27494b69f7b77c3c1598923789bfada5c145d31d0893fae1ee895213df` |
| [camera-lifecycle POM](https://dl.google.com/dl/android/maven2/androidx/camera/camera-lifecycle/1.6.2/camera-lifecycle-1.6.2.pom) | 5,860 | `7562a6960db6bd8f46441c3467a2b51d33b98ece1c118ac30538dd6c54b49ce0` |
| [camera-view POM](https://dl.google.com/dl/android/maven2/androidx/camera/camera-view/1.6.2/camera-view-1.6.2.pom) | 6,900 | `c342ccc94ecaf4dbc9d9ef4b7d292eaeb2db266ee91e59229f232e13b180419d` |
| [camera-core AAR](https://dl.google.com/dl/android/maven2/androidx/camera/camera-core/1.6.2/camera-core-1.6.2.aar) | 1,213,194 | `ebd66f090414f4c12f23a55b5023aeee1b096da8a06c02a0b6fb3520c4ddadf6` |
| [ZXing core POM](https://repo.maven.apache.org/maven2/com/google/zxing/core/3.5.4/core-3.5.4.pom) | 2,806 | `502c7368ae869f15428c020a2432eec9dde7436454f3edd1816e0d4b3d0609bf` |
| [ZXing parent POM](https://repo.maven.apache.org/maven2/com/google/zxing/zxing-parent/3.5.4/zxing-parent-3.5.4.pom) | 27,796 | `9133f322d86101c5c478c08ea3d6734dc376423937ddab1aa2a06aae24fc85c9` |
| [ZXing core JAR](https://repo.maven.apache.org/maven2/com/google/zxing/core/3.5.4/core-3.5.4.jar) | 610,364 | `71de5d89341b5fcf5dd89da7f44e84d825d0e084cdf3ec77c9abe26b0f0ceb13` |
| [JNA POM](https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.19.1/jna-5.19.1.pom) | 2,030 | `911b754a03b66af0fed2bbdfdc3a86807360dd036ad3b481ef5162c685e456bf` |
| [JNA AAR](https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/5.19.1/jna-5.19.1.aar) | 525,093 | `b57125cb7d16253f0d65a80f7d3a4c3664effa711b8bdbb7f87fb572ce1624ed` |

## Native alignment boundary

For the Android app's arm64 ABI, direct little-endian ELF64 program-header
inspection found every `PT_LOAD.p_align` equal to 16,384 in these published AAR
entries:

| AAR entry | Bytes | SHA-256 | LOAD segments |
| --- | ---: | --- | ---: |
| CameraX `jni/arm64-v8a/libimage_processing_util_jni.so` | 32,528 | `0292b063fafccf734472cbb59588e756c2dbca907c72021ba6fbf126385508a2` | 3 |
| CameraX `jni/arm64-v8a/libsurface_util_jni.so` | 4,896 | `a5c9d1928ea92ec7a94dff8cf666e7739cd734198b9f0ae1d4e28ea3c86516ba` | 2 |
| JNA `jni/arm64-v8a/libjnidispatch.so` | 176,520 | `abc26e994517bcaa3309acdb0a27373864086c7569c89d3087b8626fada9ef06` | 2 |

This is a narrow artifact observation, not APK or runtime acceptance.
[Android's page-size guidance](https://developer.android.com/guide/practices/page-sizes)
distinguishes ELF alignment, final APK ZIP alignment, and testing on a 16 KiB
environment. The central lane must check the final merged arm64 native inventory
(including Rust, JNA, CameraX and any transitive native payload), final
uncompressed-library ZIP alignment, and real loading. AAR ZIP alignment is not
the final APK's alignment. No 16 KiB device run occurred for this note.

Not inspected here: camera-camera2/lifecycle/view AAR bytes, camera-video,
camera-camera2-pipe, viewfinder-core, all other resolved transitives, generated
lockfiles/notices, or a final APK. The actual POMs add camera-video and
camera-camera2-pipe 1.6.2, viewfinder-core 1.5.1, and other dependencies; the
central license/lock gate must inspect the resolved graph rather than treating
four direct CameraX pins as the whole payload. No pin was changed.
