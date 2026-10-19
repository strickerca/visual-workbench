# P: joined Paint drawing producer

Dispatch to the existing idle Paint owner
`01a15145-22a1-76b1-ba3e-d2bbe0326006` on this exact root integration tip.
Use a checkout based on the current integration branch, not the old P/main
snapshot. Preserve its old published work and receipts. Root has no outgoing
cloud-thread sender; this is the concrete dispatch handoff for the parent.

Implement the real native producer; do not return another parser or test-only
receipt factory. This grant authorizes source implementation only: no GUI/input,
capture/device experiment, paid Actions, credentials or deployment execution.
Root owns the normal native/shared trigger and UI integration in parallel.

## Exclusive P files

- NEW `host-win/crates/vw-remote-host/src/editor_probe_runner/compatibility.rs`
  plus `compatibility/helper.rs`, `compatibility/ipc.rs`, dedicated producer
  lifecycle tests and NEW `src/bin/paint_drawing.rs`. Root registers its dedicated
  `vw-paint-drawing-helper` Cargo binary and packaged hash after source handback.
- Existing `editor_probe_runner/frame.rs` and `frame/metrics.rs`: typed actual
  completed canvas-capture/decoded-RGBA borrowing and bounded private disposal.
- Existing `editor_probe_runner.rs`: only compatibility module/export and the
  required programmatic effect-owner accessor/quiet retirement hook. Reuse its
  actual Child/Job/IO/terminal producer/retained failure owners.
- Existing `editor_effect/paint.rs`: only exact typed CanvasDot/Undo/Redo result
  or source accessor needed for this producer; keep prior effect flags truthful.
- Existing `platform/shortcuts/paint_controls/experiment.rs`: add a parked-click
  entry reusing the same finite owner and exact preflight closure. Preserve
  ordinary three-event clicks. Experimental parked click is move/down/up/return
  to the original OS-observed outside-canvas point (four events). No caller
  return coordinate. Recheck both absolute mappings after provider work. Complete
  input retirement and restore the outside point before taking any sample.
- Existing `editor_probe_runner/paint_observation.rs`: only a necessary pinned
  helper identity/accessor; preserve its query-only entry and NotTested output.

Root retains `compatibility/evidence.rs` (implemented numeric evaluator), FFI
`remote_compatibility.rs`, RemoteHub/host/admission/commands/protocol, Cargo/
manifests/packaging, NativeRemoteEdit/NativeSessions/NativeRemoteCompatibility,
shared API and Desktop Main/palette. Do not overlay older versions of those.
Additional exact files require a proposed accessor/hook before overlapping.

## Closed producer and native interface

Expose opaque native `PaintDrawingPreparation` and
`CompletedPaintDrawingObservation`; no Deserialize/FFI constructor from receipt
bytes, arbitrary path, trusted flag or external sample metadata. Helpers are
opened from exact packaged path/hash pairs and stay pinned. Root supplies the
actual selected Target, retained Arc<SourceLease>, canonical native source-context
SHA256, actual capture-session ID, fresh native UUID sample/marker IDs, shared
cancellation and an `Arc<dyn Fn() -> Result<()> + Send + Sync>` currentness check.
No imported field supplies any of them. Return exact constructor/getter signatures
and root module/retirement hooks with the implementation.

`prepare(...)` is query-only and retains the actual native Brush/settings/source
witness. `run_confirmed(self)` is one-shot and is reachable
only from the dedicated owner's confirmation of that prepared operation. There
is no consent Boolean in imported data. Root will supply a scoped exclusive
idle/input barrier before preparation/confirmation; do not implement a second
grant/selector or alter transport/input guards. Check currentness before and
after every joined phase and immediately before any actual input attempt.

Preparation cannot capture or execute a dot. Import/current-controls reading cannot prepare
or execute this experiment. Completed positive evidence stays in its actual
per-session owner; never add a global positive-receipt registry.

All UIA, WGC and input run in the **dedicated helper**, never in the JVM. Do not
call the existing frame CLI's COM/UIA functions from its parent native library.
The dedicated helper performs native WGC capture and finite input in one owned
process; do not weaken the existing single-process Job policy to spawn nested
capture/effect processes. Reuse the actual source-held guards/finite owner and
pure frame validation/decoding code. Preserve the old CLI behavior.

Use a closed bounded IPC sequence: helper can request only CanvasDot, Undo,
Redo, then a final Undo.
Before either actual input it must receive a matching parent authorization;
there is no input before authorization and no arbitrary action/coordinate/key.
Freeze the parent authorization callback as:

```rust
pub enum PaintDrawingInputStep { CanvasDot, Undo, Redo }
pub type PaintDrawingAuthorize = std::sync::Arc<dyn Fn(
    PaintDrawingInputStep, &mut dyn FnMut() -> crate::Result<()>
) -> crate::Result<()> + Send + Sync>;
```

The second argument sends only the fixed bounded authorization response on the
already ready pipe. Root checks its actual scope/exclusive barrier/cancellation
and sends that response while holding its short state guard; no provider/query/
join runs under that guard. P validates the protocol phase and empty/ready pipe
so the fixed small write cannot wait for arbitrary output. Mark parent input
uncertainty **before** authorization; only the actual source-held complete
finite-input terminal for that step clears it. Missing/partial/duplicate/wrong-
phase terminals keep the actual owner. Cancellation must prevent authorization
of a later dot/history action, not merely discard its eventual receipt. Do not relax the
existing generic effect runner's conservative uncertainty policy.

The helper writes its bounded samples only into the parent's actual owned
private temp lease. After it exits, the parent obtains a fresh affirmative
process/Job retirement receipt and completed IO join, then verifies/decodes the
retained samples and evaluates them. Only that collection accessor constructs
JoinedClick with process/Job/IO completion. Both input steps can refer to that
same actual completed helper. No live helper can claim its own process retired.

## Fixed causal sequence inside the dedicated helper

1. Fresh known `paint.brush` and full settings/canvas witness; selected actual
   installed Paint source only. Refuse unknown/nonbrush tools, unsupported
   formats/color/alpha/geometry or budgets. Do not change tools/settings.
2. Two checked, stable canvas-only lossless RGBA8 explicit-sRGB WGC captures.
   Both must represent the same target/source/canvas/settings and opaque white
   pixels. Hold actual parent source/temp/helper owners through collection.
3. Guarded parked `CanvasDot`: exact absolute move/down/up/return-to-original
   outside point, explicit finite input and `LeaveForOwnedFrameFixture`. Its dot
   destination comes from the fresh native UIA address. Require pointer outside
   the canvas before baseline and at both ends of every capture; all captures
   use the same original outside point. Verify actual WGC cursor capture disabled.
   Unknown policy, moved cursor, partial four-event batch or incomplete retirement
   refuses proof. Do not count a cursor/brush preview as ink.
4. Two stable post-dot captures, at least100ms after accepted parked input and
   at least100ms apart, with unchanged source/control witnesses.
5. Guarded parked `Undo`, two stable baseline-equal captures; guarded parked
   `Redo`, two stable captures exactly equal to the post-dot change; guarded
   parked final `Undo`, two stable baseline-equal captures. Each input is freshly
   authorized/guarded, completes actual input retirement, parks outside and has
   the same100ms settle/stability conditions. A transient pointer overlay cannot
   substitute for pixels reproduced through document history. No shortcut or
   physical pen support is inferred from these mouse actions.
6. Only after the complete helper process/Job retires and IO joins, feed the
   actual retained projections into root `compatibility/evidence.rs::evaluate`.
   It independently requires completed Dot/Undo/Redo/Undo owners, matching source/
   canvas/settings, stable opaque blank baseline, cursor disabled and parked,
   changed pixels confined around/touching the actual dot, exact redo pixels and
   exact final restored pixels. Current limit is1Mi pixels, ten fixed captures,
   radius1..64. Admit capacities/encoded/decode/child scratch before work; keep
   decoding serial and return bounded refusal instead of a whole-window fallback.

Project each click's context/settings/canvas fields only after comparing the
actual joined child's target/image/before/after controls and exact native action
against the retained parent context. Copying the expected context onto unchecked
stdout is not validation. JoinedClick has no Deserialize/foreign constructor;
the actual completion accessor is its only production construction site.

Only a successful evaluator may produce `Drawing/Supported/EditorObserved/Mouse`
with its owned evidence digest. Other six aspects remain NotTested. This proves
canvas pixels restored, **not** prior undo/redo history restored; a redo entry can
remain. Never claim WindowsPointerInput, pen pressure/hover/tilt/buttons or a
physical owner gesture from this mouse experiment. Existing individual effect
receipt `physical_or_editor_pixel_effect_proven=false` and input-free frame
receipt `input_sent=false` must remain unchanged: the composed producer supplies
the causal pixel proof that neither child alone has.

After a partial input, failed currentness or uncertain retirement, preserve the
actual strong owner. Never kill a child with unknown held input or inject Undo
without fresh guards/complete prior retirement. A failed post-dot recovery must
report outstanding canvas/history state and block another experiment until
safe retirement; do not fabricate restored pixels. Private captures are outside
the repository, retained through numeric checks and safely disposed with a text
receipt. Failed disposal retains its actual owner; source currentness must not
prevent independent verified private-file cleanup after the target closes.

Return meaningful joined-owner cancellation/partial-input/source-change/capture-
failure/disposal tests. Root's nine evaluator tests validate numeric admissibility,
not the actual producer. Windows/native/ABI/app/real experiment and physical
acceptance remain deferred; use cheap focused source checks and the parent heavy
slot rule. No per-small-commit binding regeneration.

## Current root composition and adapter handback

P is now active onbd13132d. Root composed the accepted Android b1a6f94 stack
and V2ec0b15 including00f1f26, preserving both lanes' source. Do not overlay their
roots when returning the producer.

Root has authored the real common UI lifetime in `RemoteDrawingCheck.kt` and a
conditional Desktop confirmation dialog. The actual private gateway contract is
`RemoteDrawingCheckCapability.prepareDrawingCheck(expected)` returning a retained
`RemotePreparedDrawingCheck`. Preparation is query-only; `runConfirmed()` is
one-shot. `close()` returns actual `Unchanged/Restored/RecoveryNeeded/Unknown`
canvas state only after child/Job/IO/private-file retirement; failure retains
that same owner. `Restored` requires the actual numeric pixel proof. No getter is
called after foreign destruction. A recovery outcome fences this input session
even across missing observations or geometry refresh. These synthetic lifecycle
tests and UI source do not supply the concrete NativeRemoteEdit capability.

The native adapter must retain prepared owners through the actual session's
consumer/borrow lifetime while the confirmation dialog waits. A short borrow
that ends at preparation delivery cannot protect an escaped foreign owner from
session close. Root will register that owner before bulk DTO delivery, retain
failed cleanup, and join it before parent handle destruction. Native completed
evidence must be retained by that same session and actual source context before
the preparation's transient resources are retired; no global receipt registry
or receipt-import promotion. Other query-only controls reads remain NotTested.

Root's WGC path now installs session/pool RAII before fallible configuration and
reads back `IsCursorCaptureEnabled()` after disabling it; true or unavailable
refuses capture. Actual Windows compile/runtime remains unrun. P should derive
the evaluator's cursor-free projection from this checked path, with its own
actual parked-point/source/capture witnesses, never from caller metadata.

Root still owns the real sequenced phone admission/input-helper exclusion and
normal foreign adapter/registration. Return P's exact preparation/one-shot
completion/retirement APIs and actual ownership hooks promptly; its fixed parent
authorization callback above remains unchanged. Root will not expose the UI
action as supported until that actual producer, source owner, input barrier and
packaged pinned helper are wired together.
