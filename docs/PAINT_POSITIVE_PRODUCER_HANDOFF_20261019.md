# Paint drawing producer handoff (P)

Base: `bd13132df32c5f6a52850d939a62b078adf9ce43` on the root integration
branch. Candidate: `task/paint-positive-producer-20261019`. This is source work
only. The old compatibility-import candidate remains separate and untouched.

P owns `editor_probe_runner/{compatibility.rs,compatibility/{helper,ipc,tests}.rs,frame.rs,frame/metrics.rs,paint_observation.rs}`,
`editor_probe_runner.rs`, `platform/shortcuts/paint_controls/experiment.rs`, and
the new `src/bin/paint_drawing.rs`, all under
`host-win/crates/vw-remote-host/src/`. This handoff is the only additional file.
No root Cargo, package, FFI, shared, Desktop, Android, protocol, or numeric
evaluator source was changed.

## Native interface and root hooks

`vw_remote_host::editor_probe_runner` exports:

```rust
pub enum PaintDrawingInputStep { CanvasDot, Undo, Redo }
pub type PaintDrawingAuthorize = Arc<dyn Fn(
    PaintDrawingInputStep, &mut dyn FnMut() -> crate::Result<()>
) -> crate::Result<()> + Send + Sync>;
pub type Current = Arc<dyn Fn() -> crate::Result<()> + Send + Sync>;
pub struct PaintDrawingPaths {
    pub observation_helper: PathBuf,
    pub observation_sha256: String,
    pub drawing_helper: PathBuf,
    pub drawing_sha256: String,
}
pub fn prepare_paint_drawing(
    paths: PaintDrawingPaths,
    target: vw_remote::Target,
    source: Arc<editor_source::SourceLease>,
    context_sha256: String,
    capture_session_id: String,
    marker_id: String,
    sample_id: String,
    cancel: Arc<process::Cancellation>,
    current: Current,
    authorize: PaintDrawingAuthorize,
) -> crate::Result<PaintDrawingPreparation>;
impl PaintDrawingPreparation {
    pub fn run_confirmed(self) -> crate::Result<CompletedPaintDrawingObservation>;
}
```

The completed object exposes `target()`, `image()`, `tool_id()`,
`settings_digest()`, `evidence_sha256()`, `changed_pixels()`,
`changed_bounds()`, and `disposed_private_files()`. The type has no
Deserialize/FFI/receipt constructor. Positive evidence is produced only by
the existing root-owned `compatibility/evidence.rs::evaluate` after a fresh
process/Job retirement receipt, IO join, verified source/canvas/PNG projections,
and all four complete source-held finite-input terminals. The evaluator proves
only `Drawing/Supported/EditorObserved/Mouse`; root must keep the six other
aspects `NotTested`, prior history restoration unverified, and physical/pen
claims false.

Root must register `src/bin/paint_drawing.rs` as the exact Cargo binary
`vw-paint-drawing-helper` and add its exact packaged path/hash to the checked
helper catalog. Cargo currently auto-discovers it under the source filename
`paint_drawing`; that implicit target is not the required packaged name. Root
owns the dedicated session confirmation, currentness/cancellation callback,
selected actual `Arc<SourceLease>`, source-context SHA256, capture-session UUID,
fresh marker/sample UUIDs, and scoped exclusive idle/input barrier. The
authorization callback must check its own state and call the supplied fixed
one-byte writer while holding its short state guard. It must not run provider
queries/joins under that guard. Import/current-controls paths never call
`run_confirmed`.

The existing `retry_paint_observation_retirements()` also retries quiet Paint
drawing owners and independent failed private-temp disposal. Its returned
count includes those outstanding owners. Post-dot failure keeps the strong
owner and exclusive permit; root should surface outstanding canvas/history
state and must not admit another drawing experiment. No automatic Undo,
history-restore or physical-release claim is made. Explicit owner recovery is
needed before any such unresolved session can be discarded.

## Source and failure behavior

The dedicated helper keeps one installed-Paint source for all captures and
opens a fresh exact source-held finite owner for each of Dot, Undo, Redo,
final Undo. This is necessary because actual finite retirement seals an owner
against any further send. Each step emits the existing finite-retirement
terminal before its bounded click report. Parent input uncertainty begins
before each root authorization and clears only after that terminal and exact
phase/click checks. Missing, partial or wrong-phase output retains the actual
Job, pipes, pins, source and permit. Every WGC capture is canvas-only, with
cursor capture disabled and the original OS-observed pointer outside canvas;
ten explicit-sRGB RGBA8 PNGs are decoded one at a time under a 1 Mi-pixel
canvas bound. The helper is a single Job process and never launches a nested
capture child. A successful result verifies deletion of all ten private PNGs.

Independent Astra source review of the first pushed candidate `59b69c5`
identified three blockers. The follow-up source repair adds refusal-only
decoding of the actual two white/stable baseline frames before Dot, changed
post-Dot pixels before Undo, baseline-equal pixels before Redo, and exact
post-Dot redo pixels before final Undo. The final joined evaluator remains
separate. A shared authorization/termination mutex closes the race between a
parent timeout kill and a newly sent grant. A timeout closes later grants even
when uncertain input or an unresolved canvas makes killing unsafe. After a fresh actual process/Job
receipt and IO join, private PNG disposal now runs even if canvas/input remains
unresolved; the source, Job, exclusive permit and recovery fence remain held.
The repair also accounts ten decoded RGBA frames plus serial PNG/decode scratch
against an explicit 256 MiB parent ceiling, and full-frame child scratch
against the existing 768 MiB Job limit. The native decoder uses checked
allocation. The production cleanup decision has a regression test for failed
Job receipt, unfinished IO, and post-join deletion with an unresolved canvas.
These are source repairs, not a Windows compile/runtime pass.

## Proposed deferred validation rows for the root ledger

| Candidate / phase | Exact command or journey | Prerequisites | Expected outcome | Status |
|---|---|---|---|---|
| P native source | `cargo test --locked --offline --target x86_64-pc-windows-msvc -p vw-remote-host --lib editor_probe_runner::compatibility:: -- --test-threads=1` and `cargo test --locked --offline --target x86_64-pc-windows-msvc -p vw-remote-host --lib editor_probe_runner::frame::tests:: -- --test-threads=1` | Root binary registration, Windows linker/runtime and allocated heavy slot | Fixed phase, source-held terminal, authorization/kill ordering, pre-input pixel refusal, joined-owner uncertainty, source-change, bounded capture refusal, aggregate admission and private disposal tests pass | Authored, unrun |
| P Windows package/ABI | `cargo check --locked --offline --target x86_64-pc-windows-msvc -p vw-remote-host --all-targets -j2` followed by the root's exact helper hash/package verification | Root packaged binary/catalog and heavy slot | Dedicated helper and native library compile/link; exact executable pin/hash resolves | Deferred |
| P selected blank Paint drawing | Open a fresh blank white installed Paint canvas, select Brush, move OS pointer outside canvas, confirm the root-owned experiment once; observe two baseline, paired Dot/Undo/Redo/final Undo WGC captures, then inspect owner cleanup/status | Licensed installed Paint, authorized Windows editor session, packaged exact helpers, root confirmation and actual OS capture/input authority | Only stable exact pixel restoration/redo and complete Job/IO/input retirement produce Drawing/Supported/EditorObserved/Mouse; ten private captures removed; prior history/pen/physical remain unproven | Deferred; no GUI/input/capture performed |
| P refusal/recovery | With an owned disposable blank canvas, deliberately cancel before each authorization, change source/focus between phases, induce capture/color failure and partial/held input in a controlled harness; retry retirement/cleanup | Same authorized Windows target and controlled fault harness | No later authorization or false positive; actual owner/source/Job/lease retained until safe retirement; post-dot unknown canvas blocks a new experiment | Deferred; hardware/GUI gate |

Krita remains deferred. No paid provider call, device action, purchase, signing,
credential use or public release was performed.
