# Shared core boundary

This crate builds `vw_core` and contains the real UniFFI 0.32.2 metadata. Kotlin
bindings are generated from that library; ABI checksum verification stays on.
The apps use the public platform-neutral API in `apps/shared/.../CoreApi.kt`.
The protocol is never reimplemented or parsed in Kotlin editor code.

## Ownership and scheduling

Each open project owns one bounded storage worker (16 queued calls); each live
stroke owns an independent ink worker (8 queued batches). At most eight projects,
four gestures per project, four startup jobs, and four subscriptions per project
may exist. Queue exhaustion is an explicit error, never a discarded reliable
operation. The UI must retain and retry an identical sample batch on Backpressure.
One input batch holds 1–512 samples. A gesture has a 100,000-sample boundary.
Exact retry of its most recent batch is idempotent; altered/stale retries fail.

Kotlin moves conversion/allocation to `Dispatchers.Default`, then awaits native
futures. Rust explicitly offloads work to its own threads: UniFFI's suspend
generation alone is not a worker pool. Storage/export does not lock ink input.
Camera mapping is a bounded synchronous f64 call, with no I/O or shared locks.
Actual per-call latency still requires target measurement; no 2 ms claim is made.

Close shuts the project worker's store before completing. Dropping the last
project handle also stops it after in-flight work. Gestures hold a weak project
reference and cannot keep a closed store usable. Await gesture `shutdown()` before
destroying its binding handle; shared `WorkbenchStroke.dispose()` does both.
Completion follows destruction of the ink worker state and its capacity permit,
so a retained closed handle does not occupy a gesture slot. Unpublished work is
cancelled; a durable transaction already running on the project worker may finish.
The Kotlin outer non-cancellable context preserves the caller's dispatcher so
cancellation cannot skip caller cleanup on the return from native disposal.
Dispose subscription handles explicitly. State callbacks run without project/ink locks;
each subscription is latest-only and may finish one already-started callback
after unsubscribe. Kotlin exposes a conflated Flow and closes it on project close.
Shutdown has one reserved FIFO slot beyond normal admission, drains accepted
calls, runs even if its waiting future is dropped, and shares one idempotent
completion among repeated callers only after worker state destruction.
Saturating ordinary calls cannot reject close.

Create/open/begin-stroke use a bounded independent Kotlin producer. If cancellation
occurs after native creation but before caller dispatch, the continuation's
on-cancellation handoff closes and destroys the undelivered handle. Cleanup
failures prevent subsequent acquisitions and surface as Worker errors. Project
close is serialized and completes disposal even if its caller is cancelled.
New projects are built with their verified original in an owned sibling staging
directory, closed, and published without replacing an existing destination.
A crash before publication can leave staging, never a partial selected project.

Cancellation is checked before bounded CPU stages and before durable mutation.
A stroke commit seals input; cancellation while queued discards the transaction.
Once the SQLite commit begins it finishes and the caller may recover its receipt
by retrying the same gesture or reopening. Codec calls are synchronous bounded
stages: cancellation cannot interrupt the codec midway. Typed edit and undo
retries bind the original transaction bytes and preserve the accepted cursor.
Undo/redo reconstruct from accepted journal bindings on reopen without generating
every historical inverse. They remain per-device, as required by vw-ops.

## Editor and image contracts

Contours are integer 1/256 D units and form one NONZERO compound fill. Do not
alpha-blend their overlaps independently. Prediction clones a bounded builder,
returns only a disposable tail, and never changes raw samples or the accepted
sequence. Prediction refuses clones beyond 131,072 contour vertices. Pressure
curves are eight validated monotonic Bezier coordinates; empty selects default.
Highlighter commits choose or atomically create a multiply layer. Text updates
keep the original object ID and use three property operations in one transaction.

Document snapshots contain typed shapes/style/affine, ordered layer metadata,
conservative bounds, canonical ink contours and core glyph outlines. Image pixels
are requested separately, so a stroke update does not copy the background.
The 64 MiB render-response bound counts typed outlines, duplicate points, strings,
metadata, protobuf bytes and generated boxed primitive arrays before accumulation.
Text layout is reused for both bounds and draw paths.
Background pixels are full-resolution oriented RGBA8/sRGB display derivatives;
they never replace the original bytes/depth. They require an explicit color
assumption for untagged originals and a conservative memory estimate within the
caller's budget (maximum 256 MiB); oversized work fails rather than downsampling.
Exports preserve originals, color, depth and alpha unless explicitly requested
otherwise; source/export buffers are capped at 64 MiB and CPU working budgets at
512 MiB. Decoder admission reserves retained encoded bytes; result decoding
reserves render working memory; region export reserves its full-size source before
the region-based codec budget. These are image-working budgets, excluding the
already-open persistent project model. PDF/SVG page rasterization and result image UI decoding remain typed
unsupported where their modules have not supplied an implementation.

## Authenticated app session candidate

The separate `SessionApi.kt` facade now calls real protected pairing and session
objects. Windows uses the fixed current-user DPAPI store; Android's callback uses
an app-private no-backup directory, an Android Keystore AES-256-GCM key, bounded
ciphertext, cross-process locking, durable atomic compare/exchange, and explicit
plaintext-copy clearing. The installation device ID is adopted only for a new
store; reopening with a different ID fails without replacing persisted trust.
QR credentials and code strings are secret UI values and must not enter logs,
project preferences, screenshots or exports. Code pairing requires explicit full
fingerprint comparison on both live confirmation handles. Revocation is durable.

Each listener/project link owns a bounded dedicated transport runtime. Synchronous
TLS trust reads cannot occupy a UI thread or another link's Tokio executor.
At most eight runtimes and eight admitted calls per runtime exist; each project
has one link. Project closure interrupts even an idle accept. Pairing accept,
code join and bootstrap have explicit cancellation handles. The Kotlin handoff
retains ownership until delivery or close, including child registration cleanup
when cancellation wins after the result has been dispatched. Repeated facade
close calls serialize until the original cleanup completes.

Authenticated bootstrap verifies checkpoint identity/history and every original,
then closes and atomically publishes an owned sibling staging project without
clobbering an existing destination. A client persists a separate local author ID,
retains original pending transaction bytes in SQLite, restores optimistic edits
and device undo state on reopen, and installs accepted history/removes matching
outbox rows in one transaction before publishing a new view. All store access
shares the existing project worker. Absolute editor operations can require the
captured host sequence and visible state hash inside that same worker; an exact
retry keeps its original acceptance semantics.

Sessions select explicitly supplied private/loopback endpoints in D7 order: QUIC
tether, QUIC Wi-Fi, then loopback framed TCP for adb. They retain pending OPS
through reconnect and clear volatile state. Verified partial blob offsets survive
carrier reconnect while the live handle remains open. The bounded candidate admits
64 MiB per blob/checkpoint and a caller-selected bootstrap total of at most 16 GiB
(shared default 2 GiB). Host SYNCED requires the peer's durable revision/blob-ready
acknowledgement; pending/blocked counts remain separate. Ping/pong diagnostics report
observed echo distributions and a clock-offset estimate, never hardware acceptance.
Viewport outlines contain f64 D points and do not mutate either local camera.

`ProjectLink.streamStroke` takes the actual native gesture handle, binding its
resolved highlighter layer and brush to the commit. Raw batches have explicit
sample offsets and at most 512 samples. Latest-only EPHEMERAL packets are at most
1100 bytes and 120 Hz per gesture. Missing prefixes repair through bounded CONTROL
messages; no gap invents a connecting segment. Canonical vw-ink builds the peer
geometry. Gesture IDs bind provisional retirement to accepted OPS. Cancellation
also travels reliably on CONTROL, and the authoritative host persists its guard.
Object transform/style previews are typed and bounded. Four simultaneous
generations and 100,000 samples per stroke are admitted. Protocol minor 2 and
the negotiated `preview_lifecycle_v2` capability are required: only an ordered
CONTROL opening admits a monotonically numbered generation. Its opening carries
an initial prefix/state, so an EPHEMERAL update which overtakes it may safely be
discarded. Repair requests/replies bind that generation. Reliable CONTROL closure
removes admission; late updates, old openings, repair replies and duplicate closes
cannot revive it. Completed identities do not accumulate in a lifetime cache.
OPS may precede the opening, so admission and update also consult the durable
host acceptance/cancellation lookup for the authenticated author. That lookup is
not mutated by preview expiry or a `committed=true` closure. Cancellation is a
separate host-persisted operation, and an already accepted transaction wins a
cross-channel cancel race. A completed preview may remain visually for at most
one second until accepted OPS arrives; at most four visuals are retained and
new active previews may reclaim retired visual slots. Closure never refreshes
from a delayed update and is not evidence of a durable commit. Generation state
resets only with a new authenticated connection epoch.
An in-progress native stroke retains its full samples across reconnect for the
durable commit. Its volatile suffix batches are skipped in the new epoch until
a fresh offset-zero prefix or a new gesture is supplied; missing preview prefixes
never force another reconnect or fabricate a partial stroke.
Every native preview/viewport command captures a live carrier token before queue
admission. The driver discards stale tokens, including a blocked producer which
wakes after the old queue drain. RAII invalidation prevents foreign-thread status
reads and queue-send races from moving volatile commands across reconnect.

Native regressions exercise 10,240 mixed closures, a still-active early
generation through 1,023 newer closures, CONTROL/EPHEMERAL/OPS order permutations,
exact opening and repair binding, four-slot admission, duplicate retirement,
durable authority after reopen, and own accepted-ack-before-UI-finish behavior.

The integrated apps now include pairing/QR screens, NsdManager/mDNS adapters,
connection/viewport/gesture wiring, and explicit Windows connection assistance.
These software paths do not complete physical T1.09/T1.10 acceptance. The native/shared source
provides project-role queries, validated QR inspection/text envelopes, bounded
discovery, explicit local-address candidates, read-only tether route command
text, reverse acquisition of phone-created originals before host acceptance, and
new-shape previews. See `T1.10-INTEGRATION.md` for exact limits and integration
order. New phone-origin projects are not yet handed to a new Windows authority.
Partial transfer state is not persisted across process death. Automatic per-frame
UI invalidation is integrated; large-document display/performance remains
acceptance work.
Physical carrier switching, Developer-options-off/WAN-unplugged scenarios,
Keystore device proof, project/export secret scans, camera HIL, latency, and
all UI screenshot-based acceptance remain unrun. The central Windows native run
passed 169 FFI/session/streaming/host/network tests; physical IN2019 app execution
passed 65 instrumentation cases on 2026-10-03. These results cover synthetic
fixtures and do not establish the remaining physical or interoperability clauses.

Android primary API references: [Keystore key parameters](https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder)
and [AtomicFile](https://developer.android.com/reference/kotlin/android/util/AtomicFile).

## Validation and packaging

The parent build lane builds the DLLs and `vw-bindgen`, then Gradle generates
Kotlin and packages `win32-x86-64/vw_core.dll` plus `vw_host.dll` for JNA. Android
loads the arm64 library from jniLibs; `checkAndroidNativeAlignment` invokes the
pinned NDK llvm-objdump and rejects any LOAD alignment below 2**14. The Android
application must also use uncompressed native packaging with 16 KiB ZIP alignment.
Native artifacts are build outputs, not checked-in binaries. Missing artifacts
fail rather than using stale system libraries. Public JNI/UniFFI signatures are
generated together and verified by UniFFI checksums.

`vw-ffi-golden` produces reviewed synthetic cross-language fixture text. Rust,
desktop JVM and Android instrumented smoke tests use fixed IDs, clocks, source
pixels and sample arrays. Kotlin compares exact PNG bytes and hashes with Rust,
then exercises 100,000 samples through generated bindings. The Rust suite adds
backpressure, cancellation, retry/collision, prediction, close/reopen, durable
undo/redo, text identity, multiply-layer and camera boundary cases. Written tests
are not acceptance evidence until the serialized parent runner executes them.
The generic ownership fixture deterministically forces cancellation at the queued
handle-delivery boundary. Twelve real native create/open cycles and seventy-two
gestures separately stress both queued cancellation and valid inline completion;
they exceed live handle limits and verify released locks and unchanged revisions.
Both the first and final paired Windows/IN2019 runs passed. The final run uses
the refined race stress fixture; exact receipts and the distinct source bindings
are retained in `docs/evidence/T1.07.md`.

Primary dependency records: [UniFFI 0.32.2 MPL-2.0 manifest](https://raw.githubusercontent.com/mozilla/uniffi-rs/v0.32.2/uniffi/Cargo.toml),
[UniFFI futures](https://mozilla.github.io/uniffi-rs/latest/futures.html),
[Kotlin generator configuration](https://mozilla.github.io/uniffi-rs/latest/kotlin/configuration.html),
[JNA 5.19.1 Apache-2.0 option](https://raw.githubusercontent.com/java-native-access/jna/5.19.1/LICENSE),
[coroutines 1.9.0 Apache-2.0](https://raw.githubusercontent.com/Kotlin/kotlinx.coroutines/1.9.0/LICENSE.txt).
