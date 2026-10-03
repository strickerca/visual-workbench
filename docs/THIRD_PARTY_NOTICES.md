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
