# Toolchain — starter pins and verification state (2026-10-01)

The **Known-good** column preserves the inherited specification's historical claims verbatim. It is a reference, not evidence of what is installed or a claim that every version remains current. **Pinned** records the selected starter version; **Status** separates local tool verification, completed startup validation, cached packages, and deferred work. A selected dependency is not verified until Gradle resolves it and the affected build passes. The statuses below are the setup snapshot supplied on 2026-10-01; pending entries must be updated from completed validation receipts.

Keep stable releases unless a requirement needs a preview and `DECISIONS.md` records the reason. The starter uses Kotlin 2.4.20, AGP 9.3.1, and Gradle 9.7.0 within Kotlin's explicitly fully supported compatibility range. The inherited AGP 9.4.1 / Gradle 9.8.0 pair exists, but exceeds that published range. Existing local tools are reused when suitable; an installed version is not necessarily the newest global release.

## Build tools (laptop)

| Tool | Known-good | Notes | Pinned | Status |
|---|---|---|---|---|
| Git for Windows | Not listed in inherited manifest | Source control; repository commit/push state is separate from tool availability | 2.56.0.windows.1 | Installed; version verified |
| Rust | stable 1.98.1 (1.99.0 due 2026-10-01; pin whichever is the current stable when T0.01 runs) | `rust-toolchain.toml`; intended targets `x86_64-pc-windows-msvc`, `aarch64-linux-android` | 1.99.0 | Installed; workspace tests, formatting, Clippy with warnings denied, Windows hello, Android hello, and one configured-runner Android FFI test passed |
| cargo-ndk | 4.1.x | Android builds of the core; select the minimum API using `--platform 29` | 4.1.2 | Installed; runtime version verified; Android native build and device smoke passed |
| cargo-deny | latest stable | Startup license, source, and banned/duplicate dependency checks | 0.20.2 | Installed; runtime verified; Rust license/source/bans gate passed |
| Android Gradle Plugin | 9.4.1 (2026-09-18) | Selected within Kotlin 2.4.20's fully supported AGP range; requires Gradle ≥ 9.5.0 and JDK ≥ 17 | 9.3.1 | Plugin configuration, runtime dependency audit, `:android:assembleDebug`, and one device instrumentation startup smoke passed |
| Gradle | 9.8.0 (2026-09-24) | Project wrapper; distribution and wrapper JAR SHA-256 verified below | 9.7.0 | Wrapper execution, project configuration, Android debug APK, and Windows desktop Uber JAR builds passed |
| Kotlin | 2.4.20 (2026-09-07) | Compose compiler plugin uses the same version | 2.4.20 | Plugin configuration, runtime dependency audit, Android debug APK, and Windows desktop builds passed |
| Gradle application license audit | Not listed in inherited manifest | Exact runtime graph → copied Maven POM census → offline Python allowlist gate; no dependency-license-report plugin | Gradle 9.7.0 APIs; repository Python checker | Passed: 164 modules across four runtime configurations, with one bound parent POM; 46 setup-gate tests passed |
| Compose Multiplatform | 1.12.1 (2026-09-22) | Stable desktop/UI pin; startup evidence covers the tested Windows display and renderer | 1.12.1 | Runtime dependencies audited; Windows Uber JAR built and running with two native DLLs loaded; Compose/AWT scale 1.75 matched Windows DPI 168 |
| Jetpack Compose BOM | 2026.09.00 | Stable Android dependency alignment; artifact versions are determined by the BOM rather than the inherited notes | 2026.09.00 | Runtime dependencies audited, Android debug APK built, and one device instrumentation startup smoke passed |
| androidx.activity:activity-compose | Not listed in inherited manifest | Older stable starter pin; current official Activity stable is 1.13.0 | 1.11.0 | Runtime dependencies audited, Android debug APK built, and one device instrumentation startup smoke passed |
| JDK | 21 LTS | Temurin runtime `21.0.12.1+1`; installed folder version `21.0.12.101`; desktop starter uses the owner's installed JDK with its Uber JAR | Temurin 21.0.12.1+1 | Installed; runtime version verified; Android build and desktop launch passed |
| Android SDK | compileSdk 36, targetSdk 36, minSdk 29 | Reused final API 37.0 package revision 2; Build Tools 36.1.0; compile SDK correction explained below | API 37.0 revision 2; Build Tools 36.1.0; compile 37; target 36; min 29 | Packages installed; Android debug APK build and one device instrumentation startup smoke passed |
| Android SDK command-line tools | Not listed in inherited manifest | Official Windows command-line package; SDK management without Android Studio | 22.0 | Installed; package revision verified |
| Android NDK | r30 (30.0.16248370), LTS | LTS r30; Android core build, artifact alignment, and device runtime have separate evidence | 30.0.16248370 | Installed; package revision verified; Android native build and 4 KB device Rust smoke passed; APK native libraries passed 16 KB artifact alignment checks |
| Android platform-tools (adb) | 37.0.x | Use the installed owner tool; approved IN2019 device runs Android 11 / API 30 with 4 KB pages | 37.0.1 | Installed; runtime version verified; Android Rust hello, one configured-runner FFI test, and one application instrumentation startup smoke passed; app restored installed and foreground |
| Visual Studio Build Tools | 2022 or 2026, MSVC C++ workload | VS 2026 18.5.3; MSVC 14.50.35717; installed Windows SDK 10.0.26100.0; Spectre libraries not found | VS 2026 18.5.3; MSVC 14.50.35717; Windows SDK 10.0.26100.0 | Installed inventory verified; driver-specific Spectre prerequisites deferred to T0.08 |
| Windows Driver Kit | NuGet `Microsoft.Windows.WDK.x64` 10.0.28000.x (10.0.28000.2526 on 2026-10-01) plus the same-version `Microsoft.Windows.SDK.CPP` and `Microsoft.Windows.SDK.CPP.x64` | Three matching NuGet packages downloaded into `.local/cache/nuget`; package versions, sizes, source URLs, and local SHA-256 values recorded in `.local/cache/nuget/receipt.json`; driver integration belongs to T0.08 | WDK.x64 / SDK.CPP / SDK.CPP.x64 10.0.28000.2526 | Cached with matching-version receipt; not installed or integrated; driver build unverified |
| protoc / prost | prost 0.14.x | Select the protocol compiler and Rust crate when their owning phase needs them | — | Not installed; phase-specific selection deferred |
| just | latest stable | Optional; starter scripts use PowerShell | — | Not installed; optional and deferred |
| Python | current stable 3.x | Existing interpreter for coverage checks and fixture generators; official 3.14.8 supersedes this reused version | 3.14.3 | Installed; runtime version verified |
| gitleaks | latest stable (MIT — verify) | Secrets scan in `lint-all` | 8.30.1 | Installed; runtime version verified |
| WiX Toolset | the version the pinned JDK's jpackage supports | Phase 5 MSI only; app-image development does not require WiX | — | Not installed; selection deferred to packaging phase |

### Verified wrapper hashes

| Artifact | SHA-256 | Verification boundary |
|---|---|---|
| `gradle-9.7.0-bin.zip` | `84fbba45c7f4c64abc77460e1c00f541e9f960e3c7ed2538f1ede19eacd873ae` | Downloaded distribution matched the selected checksum |
| Gradle 9.7.0 wrapper JAR | `7a9ce74cff467ca1bf60a4fcd9f05185acceda4d0f382434d393e17864262c5d` | Wrapper bytes matched the selected checksum |

These hashes establish wrapper artifact identity. The application build and runtime results below were verified separately; driver builds remain unverified.

### Confirmed starter build and runtime checks

The Android `:android:assembleDebug` build passed with compile SDK 37, target SDK 36, and minimum SDK 29. Both native libraries in the APK passed arm64, ELF load-segment alignment, and ZIP alignment checks for 16 KB pages. This is artifact inspection evidence; no 16 KB device execution has been performed.

The Windows desktop Uber JAR built and is running on the owner's installed JDK 21 with two native DLLs loaded. Windows reported DPI 168, corresponding to scale 1.75; the running Compose and AWT scale values both matched 1.75. Installer and bundled-JDK packaging remain deferred to their owning phase.

Rust Android hello and one configured-runner FFI test passed on the approved IN2019 device running Android 11 / API 30 with 4 KB pages. One Android application instrumentation startup smoke test also passed on that device. The app was restored installed and foreground, and its installed APK SHA-256 matches the final inspected debug APK. S23 testing and feature acceptance remain pending; these starter checks do not complete later product phases or broader device acceptance.

The final `test-all` run passed 46 Python setup-gate tests and one Rust ABI test. Kotlin feature-test tasks reported `NO-SOURCE`, so they provide no feature-test coverage. `lint-all` passed, with warnings retained in its validation receipts.

### Application dependency license census

`generateApplicationLicenseReport` reads the actual Android debug/release, desktop runtime, and shared desktop runtime dependency graphs without changing their consumer attributes or selecting local project binary artifacts. It rejects unresolved edges, unknown components, missing configurations, and empty coverage; then queries the exact external module coordinates for Maven POMs. The report binds each copied POM to its module coordinates, source configurations, relative path, and SHA-256. When direct licenses are absent, it retrieves the explicitly declared parent chain with a maximum depth of eight and cycle guards, binding each parent POM to exact coordinates, path, and SHA-256. `checkDependencyLicenses` verifies these bindings and uses the nearest explicit license declaration against the repository allowlist. Missing metadata, unknown licenses, forbidden licenses, unbound parents, and invalid inheritance chains fail closed.

The actual `checkDependencyLicenses --write-locks` run passed on 2026-10-01 with 164 distinct external modules. Android debug and release each covered 91 modules, desktop runtime 90, and shared desktop runtime two; these counts overlap. One exact parent POM, `com.google.guava:guava-parent:26.0-android`, supplied the declared Apache-2.0 license inherited by `com.google.guava:listenablefuture:1.0`. The run completed in 115.3 seconds; its sanitized text receipt is `application-license-gate-parent-pom-inheritance-054fbe71e567421eb48d0655330eb836.log` in the setup's temporary text-log directory. All 46 setup-gate tests passed, including 21 parent-POM regression cases. This verifies the resolved Maven metadata policy at startup. Application builds and desktop launch have separate evidence above; bundled native notices require their own verification.

The census uses Gradle's legacy, maintenance-mode `ArtifactResolutionQuery` API as an isolated bridge pinned to Gradle 9.7.0. It does not use the dependency-license-report plugin. Because local file dependencies are absent from the resolved module graph, the audit also inspects declared files throughout every runtime configuration hierarchy of each graph-reachable first-party project, including shared Android and desktop declarations. These hierarchy names are recorded in the census. First-party runtime files qualify only when they are exact Java main source-set or Kotlin JVM main-compilation class/resource output paths within the owning project's build directory, with matching declared output paths of that compilation's producing tasks; their paths and task bindings are recorded separately. Additional registered output directories and other nonempty local runtime files require reviewed license metadata and fail this gate. Empty file collections with no producing tasks are recorded as zero-artifact observations and receive no license approval; the census describes the current resolution and should be rerun after builds or dependency changes. Compiler-only declarations lie outside the shipping audit: a bounded configuration probe identified Compose's `devCompileOnly` files as the project's Java/Kotlin main class outputs with matching compiler tasks, disconnected from desktop runtime hierarchies; AGP's `androidJdkImage` is likewise SDK compiler tooling. Configuration names never grant a license exception. This Maven metadata gate does not establish license coverage for native code bundled inside third-party artifacts; their required notices and bundled files need separate evidence.

### Compile SDK correction from the real build

The initial API 36 build failed AAR metadata checks: ten selected Compose 1.12.1 artifacts require compile SDK 37. The starter now reuses the already installed final API 37.0 revision 2 platform (`PreviewSdkInt=0`, no codename/beta). Target SDK remains 36 and minimum SDK remains 29. AGP 9.3 supports API 37; no preview, suppressed metadata check or plugin upgrade was needed. [AGP compatibility](https://developer.android.com/build/releases/agp-9-3-0-release-notes), [SDK setup](https://developer.android.com/about/versions/17/setup-sdk).

### T0.05 Windows pen probe (2026-10-02)

The three private probe crates reuse the exact `windows = 0.62.2` binding already
selected for T0.03 under MIT OR Apache-2.0. Added features expose pointer input,
window controls, token/process queries and window creation; no third-party
package/version was added to Cargo.lock. First-party path dependencies are
explicitly pinned to `=0.1.0`. The probe uses Rust 1.99.0, edition 2024, with the
existing two-worker Cargo limit. `build-pen-inject` runs both license gates before
compiling and binds the resulting two executable hashes to their source and
manifests for warm HIL reuse. See `docs/evidence/T0.05.md` for measured results
and remaining editor/integrity acceptance.

## Libraries

This inherited catalog lists future candidates and license claims. None of its entries is recorded as selected, installed, resolved, or verified by this table. Owning phases must select exact versions, verify source and license evidence, and record the resolved dependency graph before using them. The starter UI dependency pins are listed separately in the build-tools table above.

| Library | Known-good | License | Used for |
|---|---|---|---|
| UniFFI | 0.32.x | MPL-2.0 | Kotlin bindings (Android and JVM) |
| JNA | 5.19.x | Apache-2.0 (dual-licensed with LGPL-2.1; used under Apache-2.0) | UniFFI Kotlin runtime |
| quinn | 0.11.12 (feature `rustls-ring`) | MIT/Apache-2.0 | QUIC carrier |
| rustls | 0.23.x with the `ring` provider (not the default aws-lc-rs) | Apache-2.0/ISC/MIT | TLS 1.3 |
| ring | 0.17.x | Apache-2.0 AND ISC | Crypto provider for rustls |
| spake2 (RustCrypto) or a CPace crate | latest stable (spake2 0.4.x; 0.5.0-pre is a preview) | MIT OR Apache-2.0 | Code-pairing PAKE (T1.06b decides) |
| rusqlite (bundled SQLite 3.53.x) | 0.40.x | MIT / public domain | Project store |
| blake3 | latest stable | CC0/Apache-2.0 | Asset IDs, state hashes |
| uuid (v7 feature) | latest stable | MIT/Apache-2.0 | Identifiers |
| kurbo | 0.13.x | MIT/Apache-2.0 | Curves, geometry |
| lyon | 1.0.x | MIT/Apache-2.0 | Tessellation |
| libm | 0.2.x | MIT/Apache-2.0 | Deterministic math |
| tiny-skia | latest stable | BSD-3-Clause | Export rasterization |
| resvg / usvg | 0.48.x | Apache-2.0/MIT | SVG rendering |
| pdfium-render + PDFium binaries | 0.9.x + 156.0.x | MIT/Apache-2.0 + BSD-3/Apache-2.0 | PDF |
| qpdf | 12.4.x | Apache-2.0 | PDF redaction cleanup (PC worker only) |
| libvips (Windows only, dynamic, `cfg(windows)`) | 8.18.x | LGPL-2.1+ | PC pyramids |
| image / zune / png / jpeg encoders | latest stable | MIT/Apache-2.0 | Codecs where OS codecs aren't used |
| mdns-sd (or OS DNS-SD APIs) | 0.21.x | Apache-2.0/MIT | Discovery |
| windows (Rust) | =0.62.2 | MIT OR Apache-2.0 | Selected for T0.03 diagnostics; target-specific Windows API bindings; transitive versions pinned in Cargo.lock |
| androidx.graphics:graphics-core | 1.0.4 | Apache-2.0 | Front-buffered wet ink |
| androidx.input:input-motionprediction | 1.0.0 | Apache-2.0 | Motion prediction |
| androidx.ink | 1.0.0 (stable only; 1.1.0-alpha09 is a preview with a breaking brush API rename; the JVM build ships no Windows native) | Apache-2.0 | D6 spike comparison |
| androidx.camera (CameraX) + ZXing core | latest stable | Apache-2.0 | QR scanning for pairing (no ML Kit) |
| androidx.heifwriter | latest stable | Apache-2.0 | Synthetic HEIF fixtures on the phone (T0.09) |
| Kotlin MCP SDK (or TypeScript SDK sidecar) | latest stable | MIT/Apache-2.0 | MCP server (T2.09 decides) |
| scrcpy (server + client) | 4.1 | Apache-2.0 (its Windows bundle's FFmpeg and libusb DLLs are LGPL; listed in `third_party/LICENSES`) | Android tunnel; uses the owner's adb, never a bundled one |
| SudoVDA | latest commit, forked and pinned | SudoVDA's changes MIT or CC0 (README; no LICENSE file); Microsoft IddCx sample code MS-PL | Virtual monitor driver (separate package) |

## Target apps and services (for compatibility testing)

| App | Version guidance |
|---|---|
| Photoshop | 27.9.1 for development (27.10 has acknowledged regressions); not installed as of 2026-10-01 |
| Krita | 5.3.x or 6.0.x (free); set Windows 8+ Pointer Input for pressure |
| GIMP | 3.2.x (free) |
| Affinity | current free release |
| Photopea | web, Chrome/Edge |
| Paint | 11.2605.81.0 installed |
| Claude Code | current; channels need `claude --dangerously-load-development-channels server:<name>` during the research preview |
| Codex CLI / ChatGPT desktop | current; the App Server is experimental (the Codex desktop app is now part of the ChatGPT desktop app) |
| OpenAI Images API | gpt-image-2.5 family as of 2026-09; verify in T0.11 |

## Official sources checked on 2026-10-01

Release pages and compatibility documentation support version availability and selection. Installed versions and hashes come from the local setup measurements; a release page is not evidence of local installation. The inherited future-library and compatibility-app catalog has not been independently revalidated in this starter update.

| Subject | Official source | Selection or verification use |
|---|---|---|
| Git for Windows | [2.56.0.windows.1 release](https://github.com/git-for-windows/git/releases/tag/v2.56.0.windows.1) | Confirms the measured Git release |
| Rust | [1.99.0 release](https://github.com/rust-lang/rust/releases/tag/1.99.0), [1.98.1 announcement](https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/) | Current Rust selection and the inherited release baseline |
| cargo-ndk | [4.1.2 manifest](https://github.com/bbqsrc/cargo-ndk/blob/v4.1.2/Cargo.toml) | Exact version, Rust minimum, Apache-2.0 OR MIT license |
| cargo-deny | [0.20.2 manifest](https://github.com/EmbarkStudios/cargo-deny/blob/0.20.2/Cargo.toml) | Exact selected version, Rust minimum, MIT OR Apache-2.0 license |
| Kotlin and Gradle/AGP compatibility | [Kotlin Gradle compatibility matrix](https://kotlinlang.org/docs/gradle-configure-project.html), [Kotlin 2.4.20 release](https://kotlinlang.org/docs/whatsnew2420.html) | Fully supported Kotlin 2.4.20 range ends at Gradle 9.7.0 and AGP 9.3.1 |
| AGP | [9.3 compatibility](https://developer.android.com/build/releases/agp-9-3-0-release-notes), [current AGP API release](https://developer.android.com/reference/tools/gradle-api), [AGP 9 built-in Kotlin](https://developer.android.com/build/releases/agp-9-0-0-release-notes) | Selected AGP minimums, inherited 9.4.1 availability, and built-in Kotlin integration |
| Gradle | [9.7.0 release](https://docs.gradle.org/9.7.0/release-notes.html), [9.8.0 release](https://docs.gradle.org/9.8.0/release-notes.html), [wrapper verification](https://docs.gradle.org/current/userguide/gradle_wrapper.html) | Selected and inherited releases; wrapper integrity guidance |
| Application license census | [resolved graph API](https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/result/ResolutionResult.html), [artifact query API and maintenance status](https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/query/ArtifactResolutionQuery.html), [component result API](https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/result/ArtifactResolutionResult.html), [artifact result API](https://docs.gradle.org/current/javadoc/org/gradle/api/artifacts/result/ComponentArtifactsResult.html), [source-set outputs](https://docs.gradle.org/current/dsl/org.gradle.api.tasks.SourceSetOutput.html), [file/task bindings](https://docs.gradle.org/current/javadoc/org/gradle/api/file/FileCollection.html), [Maven license declarations and inheritance](https://maven.apache.org/pom.html) | Exact graph/POM coverage, first-party output bindings, and failure checks; missing parent evidence fails closed |
| Compose Multiplatform | [1.12.1 release](https://github.com/JetBrains/compose-multiplatform/releases/tag/v1.12.1), [compatibility and JDK minimums](https://kotlinlang.org/docs/multiplatform/compose-compatibility-and-versioning.html) | Stable UI pin and matching Compose compiler version; desktop packaging requires JDK 17+ |
| Android Compose | [stable BOM](https://developer.android.com/develop/ui/compose/bom), [BOM artifact mapping](https://developer.android.com/develop/ui/compose/bom/bom-mapping), [Activity releases](https://developer.android.com/jetpack/androidx/releases/activity) | BOM 2026.09.00 and selected Activity history |
| Temurin | [21.0.12.1+1 release](https://github.com/adoptium/temurin21-binaries/releases/tag/jdk-21.0.12.1%2B1) | Confirms the measured JDK release |
| Android SDK tooling | [command-line tools download](https://developer.android.com/studio), [sdkmanager](https://developer.android.com/tools/sdkmanager), [platform-tools releases](https://developer.android.com/tools/releases/platform-tools) | Official acquisition and package-management sources; local installed revisions are recorded above |
| Android NDK | [NDK downloads](https://developer.android.com/ndk/downloads) | Confirms r30 / 30.0.16248370 as an LTS release |
| Visual Studio and WDK | [VS 2026 release notes](https://learn.microsoft.com/en-us/visualstudio/releases/2026/release-notes), [supported WDK versions](https://learn.microsoft.com/en-us/windows-hardware/drivers/other-wdk-downloads), [WDK NuGet installation](https://learn.microsoft.com/en-us/windows-hardware/drivers/install-the-wdk-using-nuget) | WDK 28000.2526 pairs with VS 2026; cached NuGet packages do not satisfy driver installation prerequisites |
| Cached WDK/SDK packages | [WDK.x64 10.0.28000.2526](https://www.nuget.org/packages/Microsoft.Windows.WDK.x64/10.0.28000.2526), [SDK.CPP 10.0.28000.2526](https://www.nuget.org/packages/Microsoft.Windows.SDK.CPP/10.0.28000.2526), [SDK.CPP.x64 10.0.28000.2526](https://www.nuget.org/packages/Microsoft.Windows.SDK.CPP.x64/10.0.28000.2526) | Exact matching package availability and dependency relationships |
| Python | [3.14.3 release](https://www.python.org/downloads/release/python-3143/) | Reused measured interpreter; source identifies a newer maintenance release |
| gitleaks | [8.30.1 release](https://github.com/gitleaks/gitleaks/releases/tag/v8.30.1) | Exact measured scanner release |

## T0.03 diagnostics update (2026-10-01)

The selected Windows API dependency is windows =0.62.2, with publisher-declared
MIT OR Apache-2.0 license and its full resolved graph pinned in Cargo.lock.
The exact downloaded package manifest and cargo-deny gate were checked before
compilation. The workspace Rust pin meets its declared minimum Rust 1.82.
The publisher documentation is [windows-rs](https://microsoft.github.io/windows-docs-rs/)
and the exact registry metadata is [windows 0.62.2](https://crates.io/crates/windows/0.62.2).
No third-party PNG dependency was added: the small diagnostic encoder uses
bounded uncompressed DEFLATE, with independent standard-library CRC/zlib decode.
Windows GDI, DisplayConfig and Media Foundation remain OS APIs, not bundled DLLs.
Hardware transform enumeration does not verify low-latency processing or profiles.

## T0.04 pen-probe dependency (2026-10-01)

The separate Android pen probe pins `androidx.graphics:graphics-core:1.0.4`.
The [official stable release page](https://developer.android.com/jetpack/androidx/releases/graphics)
and the exact Google Maven POM were checked. The POM declares Apache-2.0.
The runtime dependency census now covers six configurations, including probe
debug and release, with 181 exact modules and one bound parent POM. Its lockfile
is `tools/pen-trace/probe-android/gradle.lockfile`. The existing AndroidX test
runner/core 1.7.0 and ext-junit 1.3.0 pins are reused for instrumentation.
The probe keeps min SDK 29, compile SDK 37 and target SDK 36. No Samsung Remote
SDK is linked; Air Actions use declarative XML and ordinary Android KeyEvents.

Graphics Core includes `libgraphics-core.so`; the Maven license gate checks its
publisher metadata, while distribution-wide native notices remain part of the
existing release license work. This diagnostic app is not a release package.
