# Owned receiver focus readiness handoff

Scope: the S23-to-Windows drawing HIL receiver, based on
`f2302cc234725fe36cb8bf48eb1c655d092e07f6`. This is a task-branch repair;
Windows compilation and actual cross-device input acceptance remain pending.

## Cause and change

The reported Windows run passed USB routing and isolated APK installation, then
exited at `owned_target_readiness` with `remote-edit control is not granted`,
before remote instrumentation or input. The controller fixture previously called
`focus_for_owner_grant` once, before its timers and message loop. Ordinary focus
refusal therefore terminated the receiver before a later ordinary focus click
could work. The runner waits for `target.json` before launching the host and
phone instrumentation, so reordering those later launches cannot repair this.
The coarse failure does not distinguish API refusal from subsequent foreground
mismatch.

The opt-in controller fixture now starts its existing paint timer, pumps at most
64 messages per poll and retries only `Ungranted` for up to 10 seconds, paced at
20 ms. This fits inside the runner's unchanged 15-second readiness limit. A
normally focused window is accepted only after original-target revalidation and
a final actual foreground query. Otherwise the unchanged production focus helper
is used. Identity changes, paint failures and quit are fatal; late success is
refused. No target identity is refreshed. No state reference crosses native
dispatch. Target publication still follows readiness. The default surface,
explicit control grant, injection predicates and actual retirement remain intact.

The new pure helper is explicitly included in `integration_artifacts.py` so the
next artifact manifest binds its bytes. No Cargo dependency, Android APK source,
Desktop classpath source, production focus helper or grant API changed.

## Cloud validation

- Sol 6.1 / xhigh implementation; independent Astra / xhigh source review:
  accepted, no blocking findings. Review corrected a shared field capture across
  native dispatch to an ephemeral read through the retained state pointer.
- `python3 tools/check_setup.py all`: three policy checks passed.
- `cargo test --locked --offline -p vw-remote-host --test hil_receiver_readiness --test guard_regressions`:
  eight readiness tests and six existing guard tests passed.
- Focused Clippy on those two integration targets with `-D warnings`: passed.
- Rustfmt on all three Rust files: passed.
- Artifact source closure: 53 unique existing extra paths, including the new helper.
- Windows MSVC cross-check stopped in the existing `ring` dependency build:
  `lib.exe` is unavailable in this Linux environment. It did not validate the
  Windows-only native wiring. No tooling installation or workaround was attempted.

The tests exercise the actual pure readiness helper: initial refusal, later
ordinary focus, already-focused click, permanent refusal, deadline/pacing, late
success, fatal errors, immutable identity and foreground loss during confirmation.
Synthetic tests do not establish actual Windows painting or cross-device input.
The 10-second policy rejects late native results; it cannot preempt a synchronous
native dispatch. The existing outer readiness guard remains 15 seconds.

## Local build and retry

1. Fetch the published task commit and review its diff against the base above.
   Preserve the existing isolated APKs, Android core and Desktop classpath receipt
   and files with their current hashes. Serialize native work in the existing
   heavy-build slot and keep the admitted toolchain/license checks in force.
2. Rebuild only the changed receiver with Rust 1.99.0:
   `cargo +1.99.0 build --locked -p vw-remote-host --bin vw-remote-guard-hil`.
   Use the existing `Invoke-VwProcess` wrapper from `tools/process.psm1` with an
   owned phase, bounded timeout and `-CleanCompilerTelemetry`; require exit zero,
   completed streams and process-tree cleanup in its retained receipt. Run the
   two focused Rust integration suites and Windows Clippy for the HIL binary.
   Rebuilding the APKs or regenerating bindings is unnecessary for this diff.
3. Create a fresh source map and fresh artifact inventory using
   `tools/remote-edit/integration_artifacts.py` and the existing pinned Java,
   classpath, APK, Android core and helper paths. Bind the new harness hash and
   new native receipt; retain at least three valid root-reviewed build/check
   receipt bindings and the actual carrier-fault build flags. Review unchanged
   artifact hashes explicitly; an inventory does not certify a build. Do not
   relabel the previous manifest with the new source or executable hash.
4. Retry the same `balanced` scenario through
   `tools/remote-edit/integration_input_owner.ps1`, preserving its retained parent
   owner, explicit `-Execute -OwnerReady`, fresh manifest/hash and the already
   selected USB interface/device/address/port arguments. Do not invoke
   `integration.ps1 -ControllerInput` without its live retained parent.
5. The exact owned receiver can now paint and process an ordinary focus click
   during the 10-second startup interval. If focus needs a user/automation click,
   use its title bar without dragging or resizing, after matching the owned
   process/window. A client-area click is deliberately retained in the journal
   and makes the existing zero-mouse-contamination input assertion fail. Do not
   discard records or relax that assertion. If focus remains unavailable, startup
   still fails without publishing a target or granting control.
6. Require actual `owned_target_readiness`, explicit grant, phone instrumentation,
   balanced receiver pointer records, both owner retirements and owned cleanup.
   Passing readiness alone does not pass drawing, physical S Pen or latency
   acceptance. Keep this repair on its task branch until parent integration.
