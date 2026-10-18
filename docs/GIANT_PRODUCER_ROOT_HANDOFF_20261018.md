# Private giant producer root composition

Code 4a87aac91b0422a7252efce919e3de7db384b893 on
cloud/giant-producer-draft-20261018 composes the narrowly reviewed af934061
producer delta and 9b05bea streaming-reader repair. New root code still requires
independent Astra review. This is a separate draft based on editing 707e940;
correction 35ad8b4/a1ad07f is separately awaiting recheck and is not included here.
No rejected editing candidate or new root producer code was promoted to main.

## What is now implemented and checked

StoreInput bundles all four PublishedProject parts before fallible history/
replica/thread setup. Actual ProjectState retains the same 32 MiB charge last;
the session permit also survives until real state destruction. The accepted final
destination is preserved on failed construction/abandoned delivery. Three actual
project-worker regressions and an end-to-end real native publication/adoption
case exercise those lifetimes, database reopening and exact original bytes.

Native Android build invokes one fixed private JVM class from the retained input
worker. Initialization pins its class/VM and exact private AndroidTileBuild class.
A bounded operation registry supplies cancellation probes; the same native worker
may consume its actual typed output once. The native and JVM reservations must
use the same process pool. withRootOutput remains locked through checked native
copying into an independently owned directory. Source/hash/geometry, owner marker,
complete inventory, bounded encoded payloads, strict codecs and original bytes
are checked. The native build stays private until Java returns after successful
joined Android cleanup and 80 MiB charge retirement. A failed close cannot publish;
its actual JVM owner remains quarantined by the existing producer runtime.
There is no foreign runner registration or public completed-directory/manifest/
approval factory. JNI names have explicit shrinking rules.

Producer verifier reads now fill fixed pre-admitted storage and probe one stack
sentinel; growth cannot make Vec append/reallocate. Three regressions supplement
the accepted store streaming-reader tests. The fixed index writer and 1 MiB touch
scope from 9b05bea remain unchanged. 64 MiB open/install and resident envelopes,
24 MiB manifest,32 MiB codec,96 MiB surface and256 MiB phone cap are preserved.

Final native scope passes 547 executions/38 result blocks with two private child
entrypoints ignored in their parent harness and run by their owning tests.
The four packages are FFI/store/assets/assets-win; these counts are not additive
to earlier differently scoped batches or the owner's 87 cases. Strict Linux
all-target Clippy and Android arm64 library/dependency Clippy pass. Fresh Linux
core/host/generator and arm64 libraries build; all four LOAD segments retain 16 KiB
alignment. One phase core binding generation ran; host bindings stayed current.
Actual compiled JVM descriptors match all five JNI declarations/calls, and all
three JNI exports exist in the arm64 ELF. This is static ABI evidence, not ART
execution. The existing Android provider adapter needed one equivalent direct
return to pass target-specific strict lint; no provider/key operation ran.

Actual shared/Desktop/Android source and tests pass 944 JVM/unit cases (460/348/
136), with seven Windows-only skips. Shared Android instrumentation source also
compiles, including 19 producer cases; app instrumentation source compiles.
Neither ran on a device. Android app lint has zero errors/60 warnings/12 hints.
Dependency policy passes; the only lock delta is the internal assets-win package
and FFI edge, with no external pin change. The existing 264-module Gradle license
census passes. Source/artifact/log hashes and corrected failures are in
`evidence/GIANT_PRODUCER_ROOT_VALIDATION_20261018.json` and its receipt excerpts.

## Remaining implementation and acceptance

This does not activate a public picker/intake or user giant import/rebuild/export
flow. Private prepare/build/publish/close adapters, actual app ownership and
opened-project/rebuild budget composition still need their coherent root batch.
Before production admission, verify actual Android JPEG/WebP subsets, all-eight
orientation output, JNI callback/cleanup failures and renderer retirement on the
named device. Opaque codec/driver/RSS, cancellation latency and performance remain
unmeasured. The pinned Windows helper/runtime/Job boundary is unexecuted; packaged
runtime approval remains absent. No source capability flag substitutes for it.
PDF runtime activation, Krita and all physical/owner/product gates remain open.
No signing, app install/launch, live provider, credentials, device identity,
paid Actions or old-worktree deletion occurred.

## Next source lane and review order

`GIANT_PICKED_INPUT_NEXT_LANE_ADMISSION_20261018.md` grants three exact native
source files for pending inputs that reserve before opening and transfer the
same two-input permit/4 MiB charge into PreparedSource. The parent assigns that
bounded source work; root retains public records, registration/bindings and
controllers. It may proceed after this producer source freezes, with heavy
builds still serialized. The reviewed capture-picker hooks can be composed in a
separate next batch. T5.01's held recapture/semantic proposals are not applied;
use only their repaired source after Astra's verdict. Air Actions later wiring
uses accepted 2a66b8. Keep each actual caller change independently reviewed before
promotion. First obtain the exact editing correction verdict, publish only that
reviewed candidate to main, then compose it into this separately reviewed draft.
