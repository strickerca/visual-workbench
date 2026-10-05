# Krita/Paint milestone checkpoint

The owner requested a checkpoint and pause on 2026-10-04. This is an unfinished
milestone checkpoint, not a personal distribution or full acceptance release.
Resume only on a new owner instruction, within the Krita/Paint scope in
`IMPLEMENTATION_PLAN.md` and D20.

Five build tasks remain: T4.03 control/guard/input lifecycle, T4.04 complete ghost
integration, the active Krita/Paint portion of T4.07, essential T4.08 controls,
and personal packaging. Streaming startup/recovery software has passed actual
S23 initial, background-return and reconnect rendering. Physical pen fidelity,
at least 30 fps, at most 80 ms p95, physical A27 and owner drawing acceptance
remain separate and unpassed.

## Validated work

- Streaming capture/keyframe/publication/decoder recovery, with actual S23
  hardware-decoder render and internal retirement evidence.
- Basic automated phone Surface Down/Move/Up through native Windows pen input,
  covering ACK and ghost fade. The strict R100 test passed; this is not physical
  S Pen or actual editor acceptance.
- Retained ownership and release-only cleanup paths; actual failed runs remain
  documented instead of being converted into acceptance.
- Desktop grant diagnostics distinguish native grant, controlling state and
  held-witness arming. Five focused JVM cases passed in a 49.336-second affected
  desktop build, reusing unchanged native and Android bytes.
- Lifecycle evidence parsing now admits the actual emitted `sha256` field.
  The old pattern deterministically discarded it and rejected the resulting
  nine-field record. The fix passed 40 Python, 30 observation/producer-contract,
  and 65 diagnostic cases. Strict causal validation remains unchanged.

The final focused retry stopped at device selection because ADB listed no devices; no host/input owner or installation was started. Corrected real-device lifecycle acceptance remains unverified. Retry and restoration details are recorded in
[the evidence index](evidence/M4-KP.md), with failures retained separately.

## Work intentionally left open

The local R106 phone-editor fixture, R107 capture-only observer, and R119 Paint
Brush point-identity correction are independently reviewed but are not admitted
or compiled in this checkpoint. Preserve their frozen local manifests and
afterimages. The Paint correction is based on actual native UI Automation proof:
the hit is a passive Image whose immediate parent is the exact Brush button.
Krita's toolbar normalization changes focus away from the canvas; the separate
read-only capture correction preserves production input focus guards.

Complete held pause/background/disconnect release coverage, actual phone-to-
Krita/Paint drawing and Undo/Redo/final Undo with whole-image restoration,
essential controls with measured semantics and provider budgets, and normal
Android/Windows personal packaging. Do not enable catalog actions merely from
control discovery. Do not infer full milestone acceptance from a harness pass.

The next affected editor build needs fresh Windows outputs and changed desktop
and Android integration Kotlin. Reuse unchanged native Android/JNI only with
exact dependency/artifact checks. Do not repeat full native Android compilation
or unchanged passing suites for administrative evidence formatting. R120's
selected JVM runner and R122's affected Windows producer are preserved local
candidates; their broader predecessor build wrappers are not the intended path.

## Preservation and continuation

Source and sanitized text evidence belong in Git. Private fixtures, device
identifiers, local artifact archives, failure details, and verification images
do not. Task-owned verification images were inspected and disposed; text hashes
and disposal receipts remain. Obsolete generated duplicates were removed only
after checking exact retained copies and references.

The owner-specific continuation handoff is saved outside the repository beside
the original handoff. It records the final commit, current local source/artifact
bindings, stopped-agent checkpoints, frozen candidate paths and exact next steps.
Use the owner's selected model and effort; do not override settings. Select only
the authorized S23, preserve unrelated apps, and serialize builds and device/
capture/input execution. Keep pending owners and destinations until actual
settlement. Deferred editors, drivers and Phase 5 remain deferred.
