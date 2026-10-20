# Surface close phase replay

This diagnostic candidate starts from `106884244ebdaee57d47da6b93fe18eb4b5afb74`.
The Windows/S23 owner reported that the isolated drawing-tool test passed all
seven persisted-object assertions, then exhausted the existing 45-second close
wait and its cleanup close wait. Cloud did not execute that device test.

No deterministic cause has been established. This candidate adds observation of
the existing close sequence; it does not repair a demonstrated deadlock or pass
Surface acceptance. Close ordering, mutation admission, local-view final writes,
renderer/native retirement, error handling and existing wait limits are retained.

## Source trace

- `navigate(Projects)` takes `mutations` and awaits `closeCurrent()`.
- `workflows.detach()` closes the LocalViews wake channel and joins its final
  drain. `NativeLocalViews.change()` calls the native project worker directly;
  the inspected path does not reacquire the controller's mutation mutex.
- `EditorDocuments.detach()` joins document tasks, takes its document gate and
  awaits `CanvasRetirementOwner.retireAll()`. The Surface renderer's real release
  callback posts to main and resolves the awaited deferred. Awaiting it suspends
  the caller; this alone does not prove a main-thread blocking cycle.
- Native local-camera persistence stores all six binary64 values as IEEE bits.
  The inspected source does not support a normalization-driven repeated save.
- The refresh observer can still publish until later in close. Its presence is
  a source-supported race lead, without evidence that it caused this timeout.
- A second navigation can queue behind an active close if admission permits it.
  The cleanup timeout therefore does not establish a second independent fault.
  A promptly reported close error can also leave Canvas selected and exhaust the
  test's screen predicate. The next replay must distinguish failure from a wait.

## Diagnostic contract

`EditorController.closeProgress` exposes one immutable, main-confined snapshot:
synthetic attempt number, fixed stage, fixed status and optional first failed
stage. Attempt labels wrap at `Int.MAX_VALUE`; actual observation ownership uses
the attempt object. A queued retry cannot overwrite the first active close's
snapshot. Each unfinished attempt retains only its latest snapshot. Finishing
the active attempt immediately promotes and logs the oldest unfinished
successor's retained phase, including a successor already suspended in a join.
Every finished attempt leaves that registry; no completed-event history is
retained. The probe owns no coroutine, timeout, resource or admission permit.

The controller also emits fixed `VW_EDITOR_CLOSE` records:

```text
attempt=1 stage=Workflows status=Running failed_stage=None
attempt=2 stage=Queued status=Running failed_stage=None
```

Records contain no project/device identifiers, paths, drawing data or exception
messages. Observer exceptions cannot change close results. A stage is `Settled`
only after its existing body returns. Failure and cancellation propagate the
same throwable; outer wrappers retain the first failing stage. `Completed`,
`Declined` and `Failed` remain distinct. Post-storage project-list refresh and
instruction cleanup are observed before the final navigation result.

The local harness does **not** read logcat. Its finite timeout reporter must read
`editor.closeProgress` once on main and append these exact fields to the existing
instrumentation timeout text, using the captured snapshot:

```text
close_attempt=1 close_stage=Workflows close_status=Running close_failed_stage=None
```

Before any attempt the getter is null; emit exactly:

```text
close_attempt=0 close_stage=None close_status=None close_failed_stage=None
```

Live attempt bounds are 1 through 2147483647. Live status values are `Running`,
`Settled`, `Failed`, `Completed`, `Declined`; `None` is only the initial status
sentinel. The local owner confirmed schema compatibility and owns reporter
wiring. This production branch alone does not add the instrumentation suffix.

This first probe separates the existing outer joins. `Workflows` and
`DocumentOwner` each encompass their own nested retirement chain. If a replay
stalls inside one of those stages, collect the corresponding coroutine/thread
dump or add a narrowly scoped nested probe before choosing a behavior change.

## Validation and replay

The source-bound receipt is
`evidence/SURFACE_CLOSE_PHASES_20261020.json`; the exact parser vocabulary is
`evidence/SURFACE_CLOSE_PHASE_SCHEMA_20261020.json`. Forty-two focused JVM cases pass
(ten new diagnostic, four renderer, nine workflow and nineteen LocalViews).
The diagnostic cases exercise held
completion, a queued close, ownership handoff, nested failure, failure after
storage close, cancellation with retained retirement, declined navigation and
observer failure. The successor-interleaving regression fails against the
preserved original probe: it reports the first attempt completed while the
second remains suspended in document retirement. The corrected probe promotes
that second attempt immediately. This establishes the diagnostic defect and
does not establish the original S23 timeout's cause.
Existing renderer, workflow and LocalViews ownership tests are
included. The edited controller itself is compiled against cached Android/shared
dependencies. Four compiler warnings point to unchanged baseline code; the new
helper and tests emit none. This is not an APK, JNI, renderer or S23 execution result.

The Windows owner retains main promotion and real S23 replay. Replay the exact
drawing-tool test with its current object-count assertions and 45-second bound.
Retain the first active attempt's last phase, any first failed stage and queued
attempt records. Bind the result to the exact source/APK. A passing diagnostic
replay alone does not prove the original cause was fixed; use the observed phase
to select a minimal repair and deterministic regression before acceptance.
