# Lane R: canonical output composition handoff

Working base and grant: `47f2b82da1799a8481af7e92998f5ea56dc9c087`,
`ROADMAP_ROOT_HOOK_GRANTS_20261018.md` section R. Scoped source candidate:
`7fe5b235011de9e6247081a54d3b09d82dbecc7f` on
`codex/root-hook-r-20261018`, pushed to
`https://github.com/strickerca/visual-workbench.git`; `git ls-remote --heads`
returned the same SHA. No main merge, PR merge or deployment occurred. No
`.agents/skills` files were present at the checkpoint; `AGENTS.md` and the
editing/output handoff supplied the local guidance. This worker changed only
the R-granted source/test paths in the source commit. This handoff is the one
extra documentation path reported to the coordinator. The shared deferred
ledger remains coordinator-owned.

The existing `ImageOutputFrame` is now exported by `vw-geom`; the existing
typed output layer and capability are exported by `vw-model`. Model validation
admits only the dedicated kind and checks even disabled frames. Legacy
documents with no dedicated frame retain their prior source-size allowance.
Canonical D objects, original assets, document dimensions, operations and undo
are not rewritten by these registration hooks.

`vw-raster` now exports the accepted `DocumentOutput`,
`ImageOutputMetadata`, reservations, D-space editing-region renderer,
`prepare_document_output`, `render_image_output[_region]`,
`requires_document_output` and `export_document_output` APIs. Its ordinary
`render_buffer` uses the same compositor in marked mode; clean mode applies
visible tonal adjustments in existing layer order while omitting annotations
and results. The dedicated output metadata layer never draws, even when guides
are requested. Output preparation applies the single D-to-O frame to composed
pixels, preserving the original profile, depth and source identity subject to
existing explicit codec policies and budgets.

`prepare_export_pixels_with_output(source, request, output, cancel)` validates
the complete O-grid binding before preparing an optional O-region, then embeds
the immutable D extent and full D-to-O map in `ExportMetadata`. The legacy
preparer and `export` still pass `None`; optional metadata deserializes from
older receipts and is omitted from ordinary serialization. The in-memory
encoder writes provenance before PNG/JPEG/WebP encoding. The legacy document
PNG stream refuses an active output frame after project/source validation and
before reading pixels or writing bytes. Disabled output settings keep the
legacy stream path. No new compositor, codec, source owner or persistent store
was introduced.

## Focused execution receipts

All commands below used the checkpoint source plus this R source candidate,
Rust 1.99.0, two Cargo workers and the shared heavy-build lock for Cargo
tests/Clippy. The runner is Linux, so Windows `build.ps1` execution remains a
final-tree check. Text logs remain in `/tmp/vw-root-r-checks/` for the current
workspace session; their digests bind the summarized outcomes below.

| Command | Result |
|---|---|
| `python3 tools/check_setup.py all` | PASS: IDs, bundled third-party manifest and workspace policy; log SHA-256 `f5e6320331b58761322e9b6bfef0e523b47fb87d7ef78df46fe56070d8eab197` |
| `cargo deny --locked check licenses sources bans` | PASS: bans, licenses and sources; log SHA-256 `dfd54bc7d3c4c31423166c6e12bf96cdd79fc6ea80d86bfa80865eb92dce49d8`. The first offline attempt failed solely because the local crates.io index lacked `pdfium-render`; the locked online retry passed. Offline failure log SHA-256 `1e2d44c425ede11db7a064fbafcdb0078c27b9004f58485e45801f885ee981f2`. |
| `cargo test --locked --offline -p vw-geom -p vw-model -p vw-raster --test image_output -j2 -- --test-threads=1` | PASS: 6 geom + 7 model + 17 raster = 30 accepted cases; log SHA-256 `409d0515cf2e7416473d7331ad64deaddb0c0ea43b39782f252f326799dddccd`. |
| `cargo test --locked --offline -p vw-raster --test output_composition --test rendering --test streaming --test formats --test ordinary_format_preparation --test visibility_admission -j2 -- --test-threads=1` | PASS: 5 new output cases + 52 selected established raster cases = 57; one explicit 50-megapixel scratch case ignored; log SHA-256 `8f8d591544721c553bd7c0d3dab435c841c99c4e3f36710ff2a951d61b520ce9`. |
| `cargo test --locked --offline -p vw-model --test image_output -j2 -- --test-threads=1` | PASS: 8 model cases, including one new legacy-source-size regression; log SHA-256 `d1c17291df3207092fcc32c023753eb5c042890427c8dd0f2a7e95e0f96a5614`. This supersedes the earlier 7-case model run, giving 88 unique focused passes. |
| `cargo fmt -p vw-geom -p vw-model -p vw-raster -- --check`; `git diff --check` | PASS; rustfmt log is empty, SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. |
| `cargo clippy --locked --offline -p vw-geom -p vw-model --lib -j2 -- -D warnings` | PASS; log SHA-256 `dbe4ad75bccf8160d0f1d705e347dc0958c7fe54b0730cfe0509c1aa7f0df2be`. |
| `cargo clippy --locked --offline -p vw-geom -p vw-model -p vw-raster --lib -j2 -- -D warnings` | FAIL at the preaccepted `vw-raster/src/image_output.rs:199` (`clippy::manual_checked_ops`), outside the R edit grant; exact failure retained under SHA-256 `5ebc1a51611af70e6561fb367b8c598e4d8ef5dc496fb88ba8e6bb0158bfd067`. No strict raster Clippy pass is claimed. |
| Staged patch `gitleaks stdin --redact --no-banner --config .gitleaks.toml` | PASS: no leaks; log SHA-256 `76d4d6ce939f517b70ef636dddf625a7fd3a95656504ce576efdf9093539fd85`. |

An independent Astra source review of exact SHA `7fe5b235011de9e6247081a54d3b09d82dbecc7f`
against the checkpoint found no lane R semantic blocker or additional defect.
It separately confirmed the unchanged leaf Clippy policy blocker below. This
was source inspection only; the reviewer did not rerun tests or execute a
platform/device journey. Buffered codec cancellation is checked around
synchronous encoding, not proven to interrupt inside the codec.

## Exact path request and integration interfaces

One minimal out-of-grant repair is needed for strict raster Clippy:
`core/crates/vw-raster/src/image_output.rs:199-203`. Replace the guarded
division with `(*sum + sums[3] / 2).checked_div(sums[3]).unwrap_or(0)`;
this keeps the zero-alpha result and removes the newly exposed lint. The
accepted leaf was left untouched. The source compiled and all selected tests
passed; this is a policy gate, not a demonstrated rendering failure.

The coordinator still owns advertising `vw_model::IMAGE_OUTPUT_CAPABILITY` in
session admission, checking it before new layer synchronization, the reserved
`image_frames` FFI registration, filtering output editing metadata in queries,
root editor/Main viewport hooks and the final shared ledger. F can call the
public `prepare_export_pixels_with_output` for seekable codecs and
`prepare_document_output`/`requires_document_output` for current project
routes. O can use `DocumentOutput.image` and its optional `frame` plus the
public `ImageOutputFrame` matrix for package pixels and instruction geometry.
`export_with_output` is crate-private per the accepted handoff; the public
project-aware in-memory export is `export_document_output`.

## Proposed central ledger additions (coordinator to integrate)

| Requirement / phase | Candidate | Exact deferred command or journey | Prerequisites | Expected result | Status |
|---|---|---|---|---|---|
| T3.05/06 output final Rust graph | R `7fe5b235` plus final F/O/root candidate | `cargo test --locked -p vw-geom -p vw-model -p vw-raster -p vw-ffi -p vw-package --tests -j2 -- --test-threads=1`; `cargo clippy --locked -p vw-geom -p vw-model -p vw-raster --all-targets -j2 -- -D warnings` | All delegated source hooks composed; accepted-leaf line 199 Clippy repair; one heavy slot | Canonical D edit and single O export/package map agree; exact dimensions, provenance, ICC/depth, region and refusal tests pass; zero warnings | Deferred aggregate; R focused 88 unique pass, one large scratch ignored, strict raster Clippy currently fails at accepted leaf |
| T3.05/06 large output/stream budget | R `7fe5b235` plus final graph | `cargo test --locked -p vw-raster --test streaming fifty_megapixel_png16_spool_marked_export_with_256mib_budget -- --ignored --exact --test-threads=1` | Serialized heavy slot, task-owned scratch and sufficient disk/memory | 50-megapixel 16-bit stream respects 256 MiB charge, scratch and no-clobber failure ownership; no source mutation | Deferred; ignored in focused run, no large scratch allocated by R |
| T3.05/06 actual caller/viewport journey | Final R/F/O/root and platform callers | Windows `pwsh -NoProfile -File ./build.ps1 test-raster`; shared/Desktop/Android ImageFrames tests, then open source → crop/quarter-turn/resize/tone → marked/clean preview and PNG/JPEG/WebP/TIFF/BMP export → undo/redo/reopen | Final ABI, F seekable/FFI routes, O package mapping, root controller/Main and capability hooks, authorized platform target | One current original, D coordinates and undo authority; exact O dimensions and embedded D provenance; bounded cancellation/no-clobber; no duplicate annotations or stale viewport | Deferred platform/runtime; no package/device or physical gate passed |

Krita and physical S Pen/device acceptance remain deferred. Source compilation
and synthetic tests do not pass G0–G5 or A01–A30.
