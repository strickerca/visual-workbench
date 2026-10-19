# G region follow-up: two Astra P2 findings

The source follow-up to `2f180f2ba77d1e5063a1ba95c685bd66c366c23b` fixes both
reported boundaries without changing E's delivery implementation.

- **Explicit color authority:** `GiantEditingImage.editingTile` now requires an
  `assumeUntaggedSrgb` argument with no default. The root-owned native and JVM
  patches forward it to `GiantImage.read_editing_tile`, which sets the existing
  compositor option. The false path preserves the canonical refusal for an
  untagged result; callers must obtain owner approval before passing true.
  `source_region.rs` has a result fixture exercising both paths.
- **ICC admission:** `SourceInfo::validate` no longer runs in the preliminary
  binding check. The region's fixed workspace is checked before any ICC
  inspection; the full profile allowance is then checked before semantic
  validation. A new regression uses valid and malformed tagged metadata at a
  one-byte budget and expects `RasterError::Memory`; the malformed metadata
  returns `RasterError::Color` with a sufficient budget.

Owned production edits: `core/crates/vw-ffi/src/giant_images/editing.rs`,
`core/crates/vw-raster/src/source_region.rs`, and
`apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/GiantImagesApi.kt`.
The root-owned native/JVM patch contents and README were updated in this folder.

Cheap checks: `rustfmt --check`, whitespace, staged secret scan, explicit
color-argument path inspection, and `git apply --check` for all three root
patches on their named bases. The eight raster, four native, and one Kotlin
regression methods are source-only and **unrun**. Combined Rust/JVM compilation,
UniFFI binding generation, real cache/result ownership, and device retirement
remain delegated to the shared validation slot; no product or physical gate is
claimed by this follow-up.
