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
