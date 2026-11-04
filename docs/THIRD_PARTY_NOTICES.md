# Native app attribution

The dependency gate checks the resolved application graphs and pinned Rust
dependencies. `third_party/LICENSES` separately records exact SHA-256 values and
upstream URLs for retained notice files. Android packages these texts as assets;
the desktop JAR packages the same files as resources.

- JNA 5.19.1 is used under its Apache-2.0 option. Its dual-license declaration and
  complete Apache-2.0 text are retained.
- The libffi notice comes from JNA's exact 5.19.1 source tag and is retained for
  the native dispatch library.
- CameraX camera-core 1.6.2 declares Apache-2.0 and BSD-3-Clause. It includes
  libyuv, whose BSD notice is retained from immutable upstream revision
  `aa6cedb39c87910b4c28e5c71c2121fc45fd234b`. This identifies the notice bytes;
  the exact libyuv revision used to produce Google's camera-core AAR has not been
  established. See `apps/android/DEPENDENCY_PROVENANCE.md` for the artifact
  inspection and its limits.

These additional native notices complement embedded artifact notices. Full
release attribution and final distributable inspection remain Phase 5 work.

The Android assisted-selection source backend pins standalone LiteRT API/JNI/GPU
artifacts at2.3.0. Their exact AARs contain identical LICENSE and
THIRD_PARTY_NOTICE.txt files, retained once as LiteRT-2.3.0-* with hashes in
third_party/LICENSES. The complete aggregate notice includes upstream source,
build-tool and runtime terms; retaining it does not establish that every named
component is shipped or admit every native component under the primary Maven
Apache-2.0 declaration. Final native/APK component scope and the detached
model/runtime/memory admission remain open. The local model API graph excludes
the unused Play AI-pack delivery module and its Play Core terms; no license
allowlist exception or missing-class warning suppression was added.

Phase 2 adds zstd-sys 2.1.0 / bundled Zstandard 1.5.7 under BSD-3-Clause.
The Rust wrapper, generated-bindings and native Zstandard notices are retained
separately and hashed in the manifest. Their source is the registry archive
whose checksum matches Cargo.lock. These texts enter both app packages through
the existing notice resources.

The Android TLS verifier `org.rustls:rustls-platform-verifier:0.2.0` has an
upstream POM without a license block. Its exact POM and AAR are pinned in
`tools/licenses/reviewed-maven.json`; the build resolves and hashes that artifact
before the license checker accepts its supplemental upstream declaration.
The source revision recorded by the paired Rust Android crate is
`252e25161a91af476cbea620e29d277592d27ec7`. The project selects its MIT option and
retains both MIT and Apache-2.0 notices from that exact revision. The inspected
10,295-byte AAR contains three verifier classes and no native libraries. These
checks bind downloaded bytes; an upstream signature was not verified and the
artifact has not been reproduced from source. Other coordinates or changed
POM/artifact/notice bytes cannot inherit this supplemental record.

The local agent server pins Node 24.21.0 and twenty exact npm dependency
artifacts (nine in the production graph). `tools/licenses/reviewed-npm.json`
binds the lock, SHA-512 tarball integrity, package metadata and retained notice
bytes. Installation uses `npm ci --ignore-scripts` with an isolated offline
cache and empty user/global configuration. Unknown graphs or changed legal
notices fail admission. The MCP SDK's complete notice preserves both its
Apache-2.0 code and earlier MIT contributions; optional package readmes are
excluded from the runtime stage because documentation has a separate license.

`tools/licenses/node-runtime.json` retains the official archive checksum,
detached-signature verification, release-key commit/fingerprint, executable hash
and complete bundled notice hash. The runtime ships `node.exe` and its full
notice; npm and Node's build tools are not distributed. Source/build-tool license
clauses retained in that complete notice do not identify additional shipped
executables. The complete runtime file inventory is generated before packaging;
the installed bridge then embeds the completed application image inventory.

## Generated release notices (LIC-003)

Release `THIRD-PARTY-NOTICES.txt` is generated from the resolved dependency set
by `tools/release/generate_notices.py`. It is no longer the curated list above.
The curated `third_party/LICENSES` files remain the app-embedded notices. The
generator also uses them as hash-bound supplements: the JNA/libffi and libyuv
notices, the Node runtime notice, the npm notices, and one canonical Apache-2.0
text. `tools/licenses/notice-supplements.json` binds those supplements. Each
license text is copied byte for byte from the component's own package files.
The generator writes no license text.

What counts as shipped:

- **Rust:** `cargo metadata --locked --offline --all-features` for each
  product target (`x86_64-pc-windows-msvc`, `aarch64-linux-android`). The walk
  starts at the crates whose binaries ship (`vw-ffi`, `vw-host-ffi`,
  `vw-capture`, `vw-remote-host`, `vw-mcp-native`; Android: `vw-ffi`) and
  follows normal edges only. Dev- and build-dependencies are excluded.
  Proc-macro crates run on the build host and are not traversed. Workspace
  crates are first-party. Crate bytes come from the registry cache archive
  whose SHA-256 equals the `Cargo.lock` checksum. `all-features` matches
  `deny.toml`, so the set is a superset of any one build.
- **Gradle:** the census `dependencies.json` that `generateApplicationLicenseReport`
  writes for `checkDependencyLicenses`, read through the existing
  `check_gradle_licenses.read_pom_report`. Only the `:android` and `:desktop`
  (plus `:shared:desktop`) runtime configurations count. The bench, probe and
  spike apps do not ship. Artifact bytes come from the read-only Gradle cache.
  They are checked against the module-metadata SHA-256 or the cache's SHA-1
  directory name. Redirect, dependency-only and BOM modules have no artifact of
  their own and are excluded.
- **npm:** production entries of `mcp/package-lock.json` admitted by
  `check_npm_licenses.py`. The 11 dev-only packages are excluded.
- **Bundled binaries:** the Node runtime record. When `--desktop-image` names a
  built app image, the generator also copies the jlink runtime's `legal/` files
  under the documented Java runtime exception. The 125 unadmitted
  model/runtime/scrcpy proposals under `docs/license-proposals/` stay out,
  because no shipped artifact contains them.

Unknown or forbidden licenses (anything outside the `check_setup.py` allow list)
fail the run. So does a missing license text, unless the license is Apache-2.0,
whose text carries no holder-specific notice. A bundled native library without
any package notice also fails. The run is still written for diagnosis, with
status `incomplete` and exit code 2. `tools/release/assemble.py` refuses any
incomplete bundle.

The generator needs the Gradle census report. Run it after `build.ps1
license-check`, which runs Gradle `checkDependencyLicenses`. For desktop
deliveries, run it after the app image is built, so the Java runtime is
covered:

```powershell
python tools\release\generate_notices.py --output .local\notices-<release> --gradle-report apps\build\reports\dependency-license\dependencies.json --desktop-image apps\desktop\build\compose\binaries\main\app\VisualWorkbenchDev
```

The generator also scans the bytes of bundled native libraries in Maven
artifacts, read-only, for known embedded third-party code: zlib, libpng,
libjpeg-turbo, FreeType, HarfBuzz, ICU, expat, Wuffs and libwebp. A component
stays unresolved until one of its shipped texts covers each detected library.

The 2026-11-04 offline run used the existing census report and local caches,
without the desktop image. Status: `complete`, 0 unresolved. It found 425
shipped components: 273 Rust, 142 Maven, 9 npm and 1 bundled (Node), with 202
distinct texts. Output sha256 is `e877f668…a229`, deterministic across runs.
Owner-approved texts were added to
`third_party/LICENSES` on 2026-11-04:

- `notices/MPL-2.0.txt` (sha256 `3f3d9e00…9b04`), from
  https://www.mozilla.org/media/MPL/2.0/index.txt. It covers the six
  unmodified `uniffi*` 0.32.2 crates. A supplement note in the notices file
  gives the MPL-2.0 3.2(a) Source Code Form location: mozilla/uniffi-rs at
  commit `7bc621ee9d0d15371d792612707ba5ed4111f950`.
- `notices/Skia-m150-1f14f1166a-LICENSE.txt` (sha256 `5f787c1d…8dbd`), from
  JetBrains/skia commit `1f14f1166a847cb0e148f981600bf1e3e6ca6e46` (tag
  `m150-1f14f1166a`, which skiko v0.150.1 builds against).

Other resolutions:

- `libwebp-sys` 0.14.4 is a declaration-only entry, by owner decision. The
  upstream repository at the crate's commit publishes no license text, so the
  declared MIT is recorded without an invented copyright line. The crate's
  vendored libwebp `COPYING` (BSD-3-Clause) and `PATENTS` files ship.
- The `androidx.graphics` graphics-core 1.0.4 and graphics-path 1.0.1 native
  libraries are AndroidX's own Apache-2.0 JNI code. A `native_review` records
  this; the review was against `androidx-main`, not a pinned release revision.

- `org.jetbrains.skiko:skiko-awt-runtime-windows-x64` 0.150.1: the read-only
  byte probe of its DLL found libjpeg-turbo, libpng, zlib (1.3.0.1), HarfBuzz,
  ICU data, expat and Wuffs. FreeType and libwebp were not detected. Their
  license texts were retained on 2026-11-04 (owner-approved) at the revisions
  that JetBrains/skia `1f14f1166a` DEPS pins, as `third_party/notices/Skia-m150-*`.
  The source URLs are in `third_party/LICENSES`. A library whose retained text
  is generic (Wuffs ships plain Apache-2.0) counts as covered by the name it is
  registered under.

**Owner decision 2026-11-04:** the global allow list, in both
`tools/check_setup.py` and `deny.toml` (kept identical), gains
`MIT-Modern-Variant` (HarfBuzz "Old MIT"), `IJG` (libjpeg-turbo, with
BSD-3-Clause and Zlib) and `libpng-2.0`. The decision also classifies the ICU
LICENSE as Unicode-3.0. That file's GPL-with-exception sections cover
build-time autoconf files (for example `aclocal.m4`, ICU4C only) that are not
compiled into the DLL. This widening also applies to Rust crates through
cargo-deny. `docs/BUILD_SPECIFICATION.md` still lists the earlier license set.

Adding files under `third_party/notices` changes the notice resources packaged
into both apps (the desktop resources and the Android assets). The APK and
desktop image must be rebuilt before the package notice checks pass. Java
runtime coverage needs a built desktop image.
