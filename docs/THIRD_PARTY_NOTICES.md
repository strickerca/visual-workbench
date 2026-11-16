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

Phase 2 adds zstd-sys 2.1.3 / bundled Zstandard 1.5.7 under BSD-3-Clause.
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
Every license text is copied byte for byte from the component's own package
files, its pinned toolchain, or a hash-bound retained copy registered in
`third_party/LICENSES`. The generator writes no license text.
`tools/licenses/notice-supplements.json` holds the reviewed supplements. It is
parsed strictly: duplicate keys and unknown fields are refused, and each row
type has a fixed schema.

**Where the notices go:** the generated notices and their receipt are delivered
at the top of the assembled release directory. They are not inserted into the
APK or the desktop image, which carry the curated `third_party/notices`
resources. The release directory is treated as satisfying LIC-003 unless the
owner decides otherwise.

### What counts as shipped

- **Rust:** `cargo metadata --locked --offline --all-features` for each
  product target (`x86_64-pc-windows-msvc`, `aarch64-linux-android`), both
  required. The receipt binds each metadata JSON by hash and records whether
  this run generated it or it was supplied. The JSON itself does not record its
  flags, so they are checked from content:
  - every workspace member resolves with all of its declared features
    (`--all-features`);
  - no dependency edge has only target expressions that are false for the
    triple (`--filter-platform`);
  - every package and edge exists in `Cargo.lock`.

  These checks are necessary, not sufficient, conditions for the required
  command. On real data they reject both unfiltered and default-feature
  metadata.
  - The walk starts at the crates whose binaries ship (`vw-ffi`, `vw-host-ffi`,
    `vw-capture`, `vw-remote-host`, `vw-mcp-native`; Android: `vw-ffi`) and
    follows normal edges only. Dev- and build-dependencies are excluded.
  - Proc-macro crates run on the build host and are not traversed. Workspace
    crates are first-party.
  - Crate bytes come from the registry cache archive whose SHA-256 equals the
    `Cargo.lock` checksum. `all-features` matches `deny.toml`, so the set is a
    superset of any one build.
  - Native-wrapping crates (`-sys` or `links`) always ship every license file
    found up to two directories below their root. For example, `zstd-sys`
    ships `zstd/COPYING` (zstd's GPL-2.0 alternative) beside its BSD texts.
- **Rust standard library:** std, core, alloc and their vendored crates are
  statically linked into every Rust binary. The notice is the pinned 1.99.0
  toolchain's `share/doc/rust/COPYRIGHT-library.html`, plus its MIT and
  Apache-2.0 texts. That aggregate covers the whole standard-library workspace,
  a superset of what links.
- **Android NDK runtime:** LLVM libunwind and compiler builtins are statically
  linked into the Android library. The notice is the pinned NDK
  30.0.16248370's LLVM `NOTICE`, under Apache-2.0 WITH LLVM-exception.
- **Gradle:** the census `dependencies.json` that `generateApplicationLicenseReport`
  writes for `checkDependencyLicenses`, read through the existing
  `check_gradle_licenses.read_pom_report`.
  - Only the `:android` and `:desktop` (plus `:shared:desktop`) runtime
    configurations count. The bench, probe and spike apps do not ship.
  - Artifact bytes come from the read-only Gradle cache. They are checked
    against the module-metadata SHA-256 or the cache's SHA-1 directory name,
    and every cached candidate is tried before a hash refusal.
  - Only runtime variants for the consuming environment count (Android or
    standard JVM).
  - An artifact that is missing from the cache, or only partly cached, is
    unresolved; it is never assumed.
  - Redirect, dependency-only and BOM modules have no artifact of their own
    and are excluded.
  - Several `<license>` rows in a POM are treated as AND. Only a reviewed
    `pom_licenses_are_alternatives` flag treats them as OR (the JNA dual
    license).
- **npm:** production entries of `mcp/package-lock.json` admitted by
  `check_npm_licenses.py`. The 11 dev-only packages are excluded.
- **Bundled binaries:** the Node runtime record. When `--desktop-image` names a
  built app image, the generator also copies the jlink runtime's `legal/` files
  under the documented Java runtime exception. When the release delivers the
  desktop image as `desktop-file` artifacts, assembly cross-checks those files
  and `node.exe` against it. An MSI (`external-package`) is opaque to assembly,
  so it gets coverage checks but not this byte cross-check. The 125 unadmitted
  model/runtime/scrcpy proposals under `docs/license-proposals/` stay out,
  because no shipped artifact contains them.

### Selection rules

- **Policy:** a license outside the `check_setup.py` allow list fails the run.
- **What counts as license text:** only full texts do. Every recognised
  license family needs markers from its body: the grant, plus a condition or
  disclaimer clause. Examples:
  - Apache-2.0: its terms-and-conditions section.
  - MIT: "shall be included". This keeps the Boost grant from being read as
    MIT.
  - zlib: "altered source versions".
  - MPL-2.0: Exhibit A.
  - BSL-1.0: its "entire statement" clause.

  A short pointer such as "Licensed under the MPL-2.0" therefore matches no
  family. Pointers ship with the package's other license files but do not count
  as text. NOTICE, COPYRIGHT, PATENTS and THIRD_PARTY files also ship but never
  count, whatever they contain.
- **Canonical Apache-2.0 fallback:** if the only Apache file is a pointer, or
  there is none, the canonical retained Apache-2.0 text is used.
  `pdfium-render` 0.9.4 is a real example.
- **Unrecognised files:** a LICENSE or COPYING whose wording is not recognised
  counts only through a hash-bound `reviewed_files` supplement.
- **Reviewed selections:** a `selected` value in a supplement must be one branch
  of the declared expression.
- **Declaration-only entries** must name the exact package files they ship,
  bound by hash.
- **Native reviews** need a structured record: source URL, revision, summary
  and review date.
- **Embedded native libraries:** bundled native libraries in Maven artifacts
  are scanned, read-only, for embedded third-party code: zlib, libpng,
  libjpeg-turbo, FreeType, HarfBuzz, ICU, expat, Wuffs, libwebp, SPIRV-Cross,
  D3D12MemoryAllocator, brotli, dng_sdk and piex. A supplement's
  `build_components` adds libraries that the build configuration proves present
  but strings cannot show. Each such library must be covered, either by the
  content of a shipped text or by an explicit `covers` registration in
  `third_party/LICENSES`. A `covers` registration is only accepted when the
  file is the full text of the license it is registered under.
- **Receipt:** skipped ecosystems are recorded, and assembly refuses them unless
  the release declares the skip.
- **Opaque artifacts:** assembly cannot classify them. A release that delivers
  any must declare `generated_notices.opaque_coverage`, which can be an empty
  list, naming the generated graphs those artifacts contain. Each declared
  graph must be in the receipt's coverage.

A failing run is still written for diagnosis, with status `incomplete` and exit
code 2. `tools/release/assemble.py` refuses any incomplete bundle.

### Running it

The generator needs the Gradle census report. Run it after `build.ps1
license-check`, which runs Gradle `checkDependencyLicenses`. For desktop
deliveries, run it after the app image is built, so the Java runtime is
covered:

```powershell
python tools\release\generate_notices.py --output .local\notices-<release> --gradle-report apps\build\reports\dependency-license\dependencies.json --desktop-image apps\desktop\build\compose\binaries\main\app\VisualWorkbenchDev
```

### Resolutions and owner decisions (2026-11-04)

The 2026-11-04 offline run used the existing census report and local caches,
without the desktop image. Status: `complete`, 0 unresolved, deterministic
across runs. It found 427 shipped components: 273 Rust, 142 Maven, 9 npm and 3
bundled (Node, Rust standard library, NDK LLVM runtime), with 213 distinct
texts. Output sha256 is `f6d33afe…fe2c`, from the hardening follow-up. The
stricter detection now also ships `bytemuck`'s LICENSE-MIT and
`constant_time_eq`'s LICENSE-MIT0 as unrecognised package files; neither is
counted as the selected text.

Owner-approved texts were added to `third_party/LICENSES`:

- `notices/MPL-2.0.txt` (sha256 `3f3d9e00…9b04`), from
  https://www.mozilla.org/media/MPL/2.0/index.txt. It covers the six
  unmodified `uniffi*` 0.32.2 crates. A supplement note in the notices file
  gives the MPL-2.0 3.2(a) Source Code Form location: mozilla/uniffi-rs at
  commit `7bc621ee9d0d15371d792612707ba5ed4111f950`.
- `notices/Skia-m150-1f14f1166a-LICENSE.txt` (sha256 `5f787c1d…8dbd`), from
  JetBrains/skia commit `1f14f1166a847cb0e148f981600bf1e3e6ca6e46` (tag
  `m150-1f14f1166a`, which skiko v0.150.1 builds against).
- The expat, HarfBuzz, ICU, libjpeg-turbo, libpng, Wuffs and zlib texts, as
  `notices/Skia-m150-*`, at the revisions that JetBrains/skia `1f14f1166a` DEPS
  pins. The source URLs are in `third_party/LICENSES`. The Wuffs text is plain
  Apache-2.0 and carries `"covers": ["Wuffs"]`.

Other resolutions:

- `libwebp-sys` 0.14.4 is a declaration-only entry, by owner decision. The
  upstream repository at the crate's commit publishes no license text, so the
  declared MIT is recorded without an invented copyright line. The crate's
  vendored libwebp `COPYING` (BSD-3-Clause) and `PATENTS` files ship; their
  hashes are bound.
- The `androidx.graphics` graphics-core 1.0.4 and graphics-path 1.0.1 native
  libraries are AndroidX's own Apache-2.0 JNI code. A structured
  `native_review` records this; the review was against `androidx-main`, not a
  pinned release revision.

**Owner decision 2026-11-04:** the global allow list, in both
`tools/check_setup.py` and `deny.toml` (kept identical), gains
`MIT-Modern-Variant` (HarfBuzz "Old MIT"), `IJG` (libjpeg-turbo, with
BSD-3-Clause and Zlib) and `libpng-2.0`. `docs/BUILD_SPECIFICATION.md` 2.5 now
lists them too. The decision also classifies the ICU LICENSE as Unicode-3.0.
That file's GPL-with-exception sections cover build-time autoconf files (for
example `aclocal.m4`, ICU4C only) that are not compiled into the DLL.

### skiko Windows DLL

The skiko Windows build configuration (skia-pack `script/build.py`:
`is_official_build`, `skia_use_direct3d=true`; Skia `gn/skia.gni` defaults at
`1f14f1166a`) also compiles in three more libraries:

- **libwebp:** `skia_use_libwebp_decode/encode`. The DLL contains WebP chunk
  tags (VP8X, VP8L, WEBP).
- **SPIRV-Cross:** pulled in by `skia_use_direct3d` (the `gpu_shared` target).
  The DLL contains SPIRV-Cross HLSL strings.
- **D3D12MemoryAllocator:** pulled in by `skia_use_direct3d`; it is recorded
  through the supplement's `build_components`.

`dng_sdk`/`piex` are off on Windows (`skia_use_piex = !is_win`), and FreeType
is off on Windows.

Their texts were retained on 2026-11-04 (owner-approved) at the pinned
revisions:

- libwebp `COPYING` and `PATENTS` at `845d5476`.
- SPIRV-Cross `LICENSE` at `b8fcf307`. It is byte-identical to the Wuffs
  Apache-2.0 text, but kept as its own file for provenance.
- D3D12MemoryAllocator `LICENSE.txt` and `NOTICES.txt` at `169895d5`.

The libwebp, SPIRV-Cross and D3D12MemoryAllocator texts carry explicit `covers`
entries. D3D12MemoryAllocator's `NOTICES.txt` adds AMD,
microsoft-directx-graphics-samples and VulkanMemoryAllocator code, all MIT; its
premake (BSD-3-Clause) entry is a build tool. With these, the 2026-11-04 run
is `complete`.

### Rebuilds still needed

Adding files under `third_party/notices` changes the notice resources packaged
into both apps (the desktop resources and the Android assets). The APK and
desktop image must be rebuilt before the package notice checks pass. Java
runtime coverage needs a built desktop image.

### Known limits (accepted 2026-11-04)

- **Artifact contents are not inspected against the graphs.** Coverage says
  which graphs were generated; assembly does not prove that an APK, MSI or
  opaque artifact contains exactly those components. The Java runtime and
  `node.exe` byte cross-check runs only for `desktop-file` deliveries.
- **The generated graphs are supersets.** `--all-features` and the
  standard-library aggregate notice include code that a given binary may not
  link.
- **Cargo metadata flag checks are content-based.** Assembly does not
  regenerate the metadata.
- **The Gradle census is consumed as written.** Artifact bytes come from the
  local Gradle cache, bound by Gradle's own hashes. The census does not record
  which variant file each configuration selected.
- **License-family detection uses text markers.** It is not full license
  matching; unusual wordings need a hash-bound `reviewed_files` entry.
- **The embedded-library probe matches strings.** A stripped library can hide
  code from it. `build_components` records libraries proven only by build
  configuration (D3D12MemoryAllocator).
- **Two reviews are not pinned.** The androidx.graphics native reviews were
  made against `androidx-main`, not the released artifacts' revisions.
- **libjpeg-turbo's IJG terms are a summary.** Its `LICENSE.md` summarises the
  IJG terms and includes the required attribution sentence; the full IJG
  README text is not retained.
- **Placement.** The generated notices sit in the release directory, not
  inside the APK or the desktop image.
