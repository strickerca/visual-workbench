# Portable tile and format recovery

Starting point: `67b0af376612f999a0ea2543ad99a98fddd69b22`. The three
portable crates here were recovered from the 23 matching entries in
`recovery/source-candidates/MANIFEST.json` at
`0d91b70558a995fa7654021de7465b1d3f615e3d`. Their recorded sizes and
SHA-256 values matched before recovery. These were preserved, unbuilt source
candidates; recovery is a new implementation review, not acceptance of the
old worktree.

`vw-assets` supplies bounded pyramid planning, exact original-bound tile
manifests, viewport scheduling and lease-accounted RAM/GPU cache reservations.
`vw-formats` admits BMP/TIFF headers before decoding, preserves the encoded
original's asset identity and profile/depth/orientation provenance, and writes
BMP8/TIFF8/TIFF16 to caller-owned staging. `vw-avif-os` admits a deliberately
restricted single still AV1 item and, on Windows, delegates actual decoding to
the installed WIC codec. It provides no AVIF encoder, HDR or multiframe
support. The existing `vw-raster::display_rgba8` conversion is the display
boundary; tile metadata's `display_conversion` declaration cannot itself
perform or prove that conversion.

Review correction: a tile request from an older viewport generation is now
discarded even when the same tile appears in the new viewport. An old
completion never becomes fresh merely because its coordinates match.

Validation, in an isolated copied-core workspace because the coordinator owns
the root `Cargo.lock`:

- `cargo test --offline -p vw-assets -p vw-formats -p vw-avif-os --jobs 2`:
  40 focused Linux cases passed (16 tiles, 15 BMP/TIFF, 9 AVIF admission).
- `cargo clippy --offline -p vw-assets -p vw-formats -p vw-avif-os --tests --jobs 2 -- -D warnings`:
  passed with Rust 1.99. Source and tests were rustfmt checked.
- Windows `vw-avif-os` source compiled with `cargo check --offline --target
  x86_64-pc-windows-msvc` using *stub* `vw-model`/`vw-raster` dependencies.
  The real copied-core Windows check stopped in `libwebp-sys` because this
  Linux host lacks MSVC `lib.exe`; neither check exercised WIC at runtime.

Coordinator integration required: add `tiff = 0.11.3` and `weezl = 0.1.10` to
the root lock (both are MIT or MIT/Apache-2.0), and add `vw-assets`,
`vw-formats`, `vw-avif-os` as appropriate to the root-owned FFI manifests.
Use the reviewed candidate `giant_import`, `format_images`, `avif_images` and
display-color paths deliberately; keep the t3-formats ordinary-format
composition and its first-page/depth notices ahead of overlapping stream
finish. Root owns shared DTOs, project/blob transfer, controllers, actual
display conversion calls, Windows producer/worker integration and publication.
Original-blob transfer, giant JPEG/HEIC routing, libvips build/worker/GPU
performance, actual Windows WIC behavior and device/runtime checks remain
separate, unverified stages.
