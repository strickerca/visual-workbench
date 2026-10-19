# Giant producer and following lane admission

This is the integration owner's interface decision after reading the published
573ddaf provider handoff, 88cb9b3 saved-view handoff and df5c62b offline-review
caller handoff. It authorizes source implementation within the delegated lanes;
it does not approve a runtime artifact, device run, feature gate or publication
of an unverified candidate to main.

## Giant producer lane

Admit the **private native** ownership interfaces proposed in
`core/crates/vw-assets/GIANT_PROVIDER_ACCOUNTING_HANDOFF_20261018.md`:
`RootOwnedPickedOriginal`, `GiantInputLimits`, `GiantSourceInfo`,
`GiantProducerPlatform`, `GiantProducerOptions`, `GiantPublishOptions`,
`prepare_original`, `PreparedOriginal::{describe,build_approved}` and
`GiantBuiltCache::{rebuild,publish}` with the exact fields/signatures printed
there. The returned owners retain the supplied process-wide SessionBudget.
Keep all new producer/input/publish implementation in dedicated giant-image
leaves. The coordinator retains root registration, public DTO admission, generated
bindings, manifests/lock and the app entrypoints.

The input File and TempDir must be root-owned before asynchronous dispatch.
A runner path is only a private description inside that owned directory: it is
not permission to reopen an arbitrary caller path. Pin the actual input/file
identity for runner reads, recheck exact source hashes/length and fence the full
project binding at rebuild/publication. A boolean consent authorizes only the
explicit untagged-sRGB interpretation; it never approves a runtime or producer.

Only an actually completed, validated platform runner can mint the private
GiantBuiltCache witness. Keep producer construction private. No persisted marker,
caller result directory, self-asserted version or public factory may mint it.
Missing or unapproved production output must continue to refuse. Preserve all8
EXIF orientations and reject unsupported profile/depth/alpha before publication.
Run bounded band/region decoding, account actual copies and keep every owner
until joined runner/renderer retirement. No whole-original decoder fallback.

AndroidRegionJpeg is JPEG-only and needs an actual bounded BitmapRegionDecoder
runner with the owned input and a native-validated output witness; do not route
PNG through it. Its dedicated platform leaves need a separately assigned owner
to avoid overlapping the current Android renderer lane. WindowsPinnedVips needs
reviewed exact runtime pins/notices, bounded job/worker shutdown, packaging and
an actual Windows execution receipt. Recover dedicated vw-assets-win source if
valuable; propose any Cargo/dependency/target registration change for coordinator
composition. Do not install binaries, alter pins or claim approval from source.
No new public FFI input callbacks are admitted merely by this private contract;
return a concrete proposed bounded callback/descriptor seam for root review.

## Saved views / document editing lane

Admit the additive LocalViewsApi.kt records and injected WorkbenchLocalViews
signatures from88cb9b3 unchanged. Admit the dedicated recovered local_views.rs
facade and NativeLocalViews.kt adaptation described by that handoff; preserve the
archive privacy deletion before registration. Root will register the named DTOs,
compose controller attachment/navigation and generate bindings in its separate
coherent batch. The lane may finish dedicated layer/tonal/rotation leaves and
propose exact shared commands before adding them; do not edit existing shared
schemas, controllers, canvas or root UI. Page-aware bookmarks remain unsupported.

## Offline review lane

Admit the additive OfflineReviewApi.kt records and WorkbenchOfflineReview methods,
dedicated offline_review.rs, NativeOfflineReview.kt, interaction and callback-only
platform panels fromdf5c62b for separate coherent composition. Existing project,
replica/history commit and OPS notification remain authoritative. Root must add
the documented History::restore conflict-review Lamport floor before registration
is made usable, then register the dedicated module and generate bindings once.
No schema replacement or additional transport/identity owner is needed.

Callbacks must run under the existing edit mutex without reacquiring it. Root
will provide identity/epoch-checked refresh/metadata hooks, detach before parent
close and admission that fences app-local drafts. Accepted durable work is never
relabeled unwritten on cancellation. Private review text stays out of logs.
Root controller/UI ownership remains reserved during the current giant caller
batch. After that source is frozen, the parent may explicitly transfer a narrow
caller composition lane; no worker should overlay old root postimages.

## Exact current boundary

Final affected native tests pass491cases, with two private subprocess entrypoints
ignored, and strict Clippy passes. The native/provider/delivery subset includes23
cases. Shared/Desktop/Android JVM tests pass816cases, seven Windows-only cases
skip; instrumentation source compiles and lint has zero errors. Approved
producer/import/rebuild, actual framework/device lifetime, target memory/performance
and physical acceptance remain open. Following-lane source admission does not count those workflows as
wired or tested, and does not broaden into phone AI/provider calls or Krita.
