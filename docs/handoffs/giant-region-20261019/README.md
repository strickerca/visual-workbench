# Giant region composition: integration hooks

Source candidate: `2f180f2ba77d1e5063a1ba95c685bd66c366c23b` on
`feat/giant-region-composition-20261019`, based on checkpoint
`134b421ba5e5732c6cbdb90b1cb73ee831f56793`. Delivery prerequisite:
`b2c874695f6ef79f8f88c62bc367e78dbdfcf026` (E). The three patches in
this directory are the complete, unapplied root-owned hooks. They change no G
production file and should be applied by the integration owner after both
candidates are composed:

1. `root-raster-checkpoint.patch` registers and exports the bounded D-space
   region compositor and reduction in `vw-raster/src/lib.rs`. Its context is
   checkpoint `134b421`.
2. `root-giant-after-E.patch` exports `GiantImage.prepare_editing_tile` through
   E's `PreparedGiantTile` in `vw-ffi/src/giant_images.rs`. Its context is E
   commit `b2c8746`.
3. `root-jvm-after-E.patch` makes the existing native giant adapter implement
   the optional `GiantEditingImage` capability. Both raw and composed tiles use
   E's prefetched publication lease, cancellation cleanup, and retryable
   retirement. Its context is E commit `b2c8746`.

Apply each with `git apply --check <patch>` then `git apply <patch>` on the
combined tree. The raster patch was checked against `134b421`; the other two
were checked against `b2c8746`. Regenerate bindings after the native method is
registered so Kotlin receives `prepareEditingTile`. Platform callers must
select committed-composition mode explicitly and suppress duplicate canonical
annotation draws. The existing raw tile mode remains independently available.

The G commit passed source formatting, staged whitespace, and staged secret
scans. Its seven raster, four native, and one Kotlin cases remain **unrun**;
compile, ABI, shared JVM, real-cache, device and renderer-retirement checks
belong to the combined validation slot. Composed levels above 2 explicitly
refuse; no full viewport, frame-rate or physical acceptance claim is made.
