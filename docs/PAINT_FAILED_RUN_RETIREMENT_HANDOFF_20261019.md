# Paint failed-run retirement handoff (P)

Base: `e13d6a541098f32a705ef40e9f83ec97b7e92615` on
`cloud/roadmap-source-integration-20261018`. Candidate branch:
`task/paint-failed-retirement-20261019`. This source change stays within P's
`editor_probe_runner.rs`, `editor_probe_runner/paint_observation.rs`, and
`editor_probe_runner/compatibility.rs` plus its dedicated tests. The latter two
generic Owner initializers change only to initialize the new private disposal
count. No root FFI, Hub, Cargo, shared, Desktop or package source was edited.

## Native adapter hook

Before consuming `PaintDrawingPreparation`, obtain its
`retirement_handle() -> PaintDrawingRetirementHandle`. The handle is cloneable,
opaque, bound to this one native preparation, and holds the actual failed
process/Job/IO/private-lease owner. Then call the existing one-shot
`run_confirmed(self)`. On failure, keep the handle and the root-owned peer/host
input exclusion. The generic `RetirementPending` return is not a canvas state.

`handle.status()` returns `PaintDrawingRetirementStatus` with actual
`canvas: PaintDrawingCanvasState`, `process_and_job_retired`, `io_joined`,
`private_files_disposed`, `owner_retained`, and the original `run_error` when
known. `handle.retire()` retries the exact owner's fresh Job receipt, finished IO
join and private-file disposal. It seals a preparation that has not started. A
running attempt returns `RetirementPending` until its native cancellation and
run method finish; root must signal the registered native cancellation token
first and join outside its state monitor.
Before a retirement query, the handle closes later authorization and attempts
the existing guarded shutdown of a safely pre-input helper. This lets a child
waiting on stdin after IO-thread startup failure retire. Unknown input or an
unresolved canvas prevents that kill and continues to hold the child.

`RecoveryNeeded` means a dot grant may have changed the actual canvas/history;
`Unknown` means input or a live launched owner is still uncertain. Neither is
cleared by Job retirement, IO join or private-file deletion. `retire()` retains
the strong source, child, pins and native permit and returns
`RetirementPending` in either state. `Restored` is set only after the joined
numeric pixel evaluator succeeds. `Unchanged` is returned after a no-dot run
has safely joined or a prelaunch failure. The root adapter must read the state
before foreign destruction, retain the same native handle when retirement is
pending, and keep its peer/host exclusion until actual recovery/retirement is
resolved. No caller flag or imported receipt can mark recovery complete.

If the last handle is dropped while an owner or private lease is still held,
the existing quiet global retirement registry takes that actual owner as a
process-lifetime safety fallback. Its count remains an unload fence, not a
per-run state query. Root should therefore retain its handle, including on
session close and cleanup failure, and should not substitute the global count.

## Focused and deferred checks

Authored Windows source regressions cover unknown active attempts, sealing a
late execution, refusal to promote unresolved/uncertain canvas to Restored,
safe pre-input shutdown before cleanup, and retry of the same partially cleaned
private lease with an exact file count.
Formatting, diff checks and offline Cargo metadata are cheap source checks;
they do not execute the Windows tests or the native adapter.

Deferred command: `cargo test --locked --offline --target x86_64-pc-windows-msvc -p vw-remote-host --lib editor_probe_runner::compatibility::tests:: -- --test-threads=1`.
Prerequisites: root-owned Cargo/package/ABI composition, Windows linker and
runtime, and the single allocated heavy slot. Expected: the new retirement
regressions and existing producer lifecycle tests pass with actual Windows
private-temp semantics. A later controlled disposable blank-Paint journey must
cancel before grant, fail after Dot, and force private-file cleanup failure;
verify the matching native handle reports `Unchanged`, `RecoveryNeeded`, or
`Unknown` as appropriate, retains source/Job/permit for unresolved state, and
blocks another experiment until real recovery. No GUI, input, capture, device
or paid provider action was performed here. Krita remains deferred.
