# Paint compatibility import: scoped source handoff

The P branch `task/paint-compatibility-import-20261018` starts at the root-hook
publication `47f2b82da1799a8481af7e92998f5ea56dc9c087`. It reapplies the
reviewed original P source as `95eb77a`, `a23232a`, and `930b576`; the new
importer source is `1738029`. The original portable P candidate remains at
`task/paint-editor-controls-20261018` (`1daa7a6`) with its earlier detailed
handoff and accepted source review. No shared roots, contracts, or central
validation ledger were edited on this branch.

## What the new source does

The actual Desktop Paint palette now offers a bounded receipt picker. The
selected regular file is read without following a symbolic link, capped at
32 KiB, checked against the still-current palette target, passed to the shared
importer, and then erased from the UI's byte array. The palette serializes
import with editor commands, retains its own read until cancellation cleanup
joins, and uses its existing `compatibility(tool, settings, records)` adoption
method. Imported records never change verified actions or command authority.

The JVM adapter strictly decodes the closed schema, UTF-8, exact Paint image,
tool/settings, aspect/method/input API/device/evidence fields, and byte count.
It compares the complete bytes, SHA-256, records, image, and target binding to
an independently held observation in `NativeCompatibilityObservationAccess`.
Raw files, digests, flags, profile metadata, helper queries and input ACKs do
not admit evidence. The native-session adapter checks the current target both
before and after borrowing. Its close path fences new borrows, cancels active
ones, and joins their cleanup before native handle retirement. The close-first
ordering is covered by a synthetic regression.

**The production observation source is still absent.** The only normal factory
at `apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeSessions.kt:146`
constructs `NativeRemoteEdit(handle, this)` without the optional owner access.
Consequently current product imports refuse and report that no observation
owner admitted them. The synthetic owned receipts in the unit cases are test
fixtures, never product evidence. This source is a safe caller/decoder candidate,
not completion of actual T4.07 compatibility observations or any physical gate.

## Exact coordinator hooks requested

1. Grant or implement the read-only joined-result handoff in
   `host-win/crates/vw-remote-host/src/editor_probe_runner.rs` and, only for
   independently observed frame effect, its `src/editor_probe_runner/frame.rs`
   owner. A new `src/editor_probe_runner/compatibility.rs` plus dedicated tests
   may hold a projected record. The handoff must retain the actual completed
   process/Job/IO owner and exact receipt bytes/digest, image/source proof,
   observed tool/settings, input API, method, device and effect evidence. The
   existing `editor_effect/paint.rs` receipts explicitly report
   `physical_or_editor_pixel_effect_proven=false`; the read-only frame receipt
   explicitly reports `input_sent=false`. Neither alone can produce a
   Supported/editor-effect/physical record. If the joined evidence cannot prove
   an aspect, return no record or a truthful Not tested record.
2. Root owns the native session/FFI registration and
   `NativeSessions.kt:146`. It must supply an implementation of the already
   compiled interface
   `NativeCompatibilityObservationAccess.withCurrentReceipt(expected,
   receiptSha256, body)` to the optional `NativeRemoteEdit` constructor. That
   implementation must borrow the actual source within the current selected
   target owner; cancel and join native work before returning/closing; project
   the closed `paint-compatibility-observations` v1 wire bytes retained by that
   owner; and independently derive matching `NativeOwnedCompatibilityReceipt`
   records. It must refuse source, identity, tool, settings or session changes.
   If the native source has no such actual evidence yet, leave the factory
   argument absent and keep the honest refusal. No catalog or arbitrary signed
   JSON can substitute for this owner.

The exact file proposal above was requested before editing those native paths.
No native probe owner, `NativeSessions.kt`, FFI, catalog, controller, or main
file was changed in P. A root registration/factory grant is required before P
can enable a nonempty real source; this branch intentionally has no trust flag
or invented record producer.

## Executed focused checks

- `source /workspace/.vw-tools/cloud-setup/start.sh && python3 apps/shared/src/commonTest/paint/run_compatibility_probe.py`
  passed: strict `-Xexplicit-api=strict -Werror` compile of exact shared/JVM
  importer leaves, 10 synthetic admission/lifetime groups, five Desktop
  bounded-file cases. The Desktop guard functions were compiled unchanged
  from source; Compose and the complete native adapter were not built.
- `source /workspace/.vw-tools/cloud-setup/start.sh && python3 apps/shared/src/commonTest/paint/run_probe.py`
  passed: exact Paint palette leaves and 16 synthetic scenarios.
- `git diff --cached --check` and a staged secret-pattern scan passed before
  the source commit. The checks used disposable temporary classes and files.

## Proposed rows for the root-maintained deferred validation ledger

| Requirement | Exact deferred command or journey | Prerequisites | Expected result | Status |
|---|---|---|---|---|
| T4.07 shared/desktop importer | `bash apps/gradlew -p apps :shared:desktopTest --tests 'com.visualworkbench.shared.RemoteCompatibilityImportTest' --tests 'com.visualworkbench.shared.RemotePaintPaletteTest' :desktop:test --tests 'com.visualworkbench.desktop.remote.RemoteCompatibilityImportTest' -x :desktop:packageConnectionRuntime -Pkotlin.incremental=false --no-daemon --no-parallel --max-workers=2` | Final composed tree, normal generated bindings, license/dependency gate and one heavy build slot | Full actual module/Compose/JVM compilation; all importer and palette regressions pass, including close-first retirement; no unauthorized command grant | Deferred; leaf compilers/cases only passed |
| T4.07 native receipt reader | `./build.ps1 test-remote-helpers -TimeoutSeconds 900` in the approved Windows host environment, followed by the root-selected native session/FFI tests | A real joined-effect owner and exact reader/factory hooks reviewed and composed; normal Windows toolchain and license gate | Source/image/tool/settings/effect records are derived only from completed owned observations; failures, read-only frames and input-only receipts refuse; cancellation joins before native close | Deferred; prerequisite implementation/access is open |
| T4.07 actual Paint import | In an owned blank matching Paint image/package, create an authorized observed-effect receipt with the native joined owner; import it in the Desktop palette; repeat with modified byte, tool/settings, image, target, link close, cancel and concurrent command | Real completed native reader/factory, current catalog, licensed Windows capture runtime, authorized exact target | Only exact current owned observations appear; unsupported and Not tested remain honest; command eligibility unchanged; close-first joins; source changes clear stale rows | Unrun; no product observation source exists in this lane |
| T4.03/04/08 and personal package | Use the original P handoff's final Paint command/grant/contact/ghost-ink and packaging commands/journeys against the final coherent tree | Coordinator composition, package/license inputs and authorized selected endpoints | Exact current action/receipt/retirement behavior, package notices/checksums, measured runtime evidence; no old-artifact reuse | Deferred; original 16 cases are synthetic only |

Krita, physical S Pen support, product acceptance, paid providers, signing,
deployment and main-branch merge remain outside this candidate. Preserve
existing drafts, recovery receipts and failure evidence during root integration.
