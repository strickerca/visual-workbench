# Paint runtime observation owner

Root continuation after `6b5dc8c` promotes the published P importer/fixes
(`95eb77a`, `a23232a`, `930b576`, `1738029`, `1b53ead`) as a review candidate,
then supplies its previously missing normal runtime owner. This is source work;
Windows execution, generated bindings and positive compatibility are not passed.

`NativeLink.remoteEditHandle()` now supplies `NativeCompatibilityRuntimeAccess`.
The pinned application runtime includes `vw-editor-probe.exe`, whose retained
path/hash configures the native host. The native observation reads the actual
authenticated hub's binding, selected target, image identity and profile digest.
It opens an actual installed-Paint parent source lease before the existing
suspended/Job-assigned/pinned helper launch. No imported field selects a target,
policy, helper or profile. Only `--observe-paint-controls` is reachable here;
there is no capture, input or experiment mode.

The query returns only after actual child/Job retirement and completed IO join.
Failed retirement uses the existing strong failure-owner registry, retaining
the parent source lease too. Quiet application owners never print UIA receipts
to stdout; native library retirement checks retry/count them and the active
probe permit. Projection skips unknown UIA trees instead of building a full
dynamic JSON tree; the existing 1 MiB output and single-helper permit remain.

The opaque foreign owner is acquired before receipt bytes and held through the
callback. Its final check verifies the same hub/binding/source/helper and queries
the actual tool/settings again. The cancellation bracket disposes an owner that
arrives after caller cancellation; normal remote close first signals the actual
hub helper cancellations, then cancels/joins receipt borrowers before releasing
the native link. Synchronous source/provider calls remain bounded and can delay
caller cancellation; no immediate interruption or Windows close test is claimed.

Known Brush/Eraser and full settings come from the actual controls/frame witness,
not profile-prescribed settings. Native generates a canonical receipt with all
seven aspects `NotTested`, `NotPerformed`, unknown input API and no evidence or
device. The palette's new **Read current Paint controls** action displays that
truthful state. Imported bytes still require exact owner-held bytes, hash,
identity, tool, settings and records; positive imported support is refused by
this producer. Observations never alter action permissions.

The causal drawing-effect producer and physical owner-gesture producer remain
missing from the normal product path. Existing isolated experiment/frame harness
outputs have not been promoted into positive compatibility authority. This is a
remaining implementation gap, separate from unperformed physical acceptance.

Focused checks: 16 Kotlin admission/ownership groups (including creation-time
cancellation and joined disposal), five Desktop file-guard cases, 16 existing
Paint palette cases, five exact Rust projection/receipt tests, 33 current
runtime/catalog JUnit cases and 33 package-report tests. Counts overlap where
noted and are not a final-tree total. Importer/palette probes use cached Kotlin
2.4.0; runtime JUnit uses 2.4.20. Rust projection uses matched cached `vw-remote`
dependencies and substitutes only the native receipt error enum. It does not
compile the Windows runner, UniFFI object or JNI factory.

Reproduce with `apps/shared/src/commonTest/paint/run_compatibility_probe.py`,
`run_probe.py` in that directory, `tools/tests/check_paint_observation_projection.py`,
`tools/tests/check_paint_runtime.py`, and Python unittest discovery of
`tools/desktop-test/test_reports.py`. See the central deferred ledger and
`evidence/PAINT_RUNTIME_OBSERVATION_20261019.json` for exact remaining checks.
