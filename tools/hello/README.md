# Toolchain smoke tests

These are setup probes, not feature or hardware acceptance tests.

* Windows Rust: `cargo run --locked -p vw-hello --bin vw-hello-win`.
* Android Rust: `cargo ndk -t arm64-v8a --platform 29 build --locked -p vw-hello --lib`.
  Inspect the resulting `.so` with the pinned NDK's `llvm-objdump -p`; LOAD
  alignments must be at least `2**14`.
* Android Compose: `build.ps1 build-android`, then `build.ps1 hil-test`.
  The starter in `../../apps/android` loads the real Rust `vw_core` library.
* Windows Compose: `build.ps1 build-desktop`, then `build.ps1 run-desktop`.
  The starter in `../../apps/desktop` loads both Rust libraries and prints
  `VW_DESKTOP_READY`, its Compose density and AWT display scale.

Both Kotlin probes reuse the actual application starter modules so the smoke
tests exercise the same build/native-packaging pipeline as subsequent work.
