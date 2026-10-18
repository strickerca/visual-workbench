# Real assisted-selection backend lane

Grant to the SAME editing owner **01a15009-f394-70eb-9e10-e873087f1003** on parent
delivery. Base: accepted main6d505416 plus exact editing source707a92c6 and portable
admission/review record9e41ed1d779b0c91a7c60d55a829c1f3e1defb28. Its Astra review
binds base6e7c017b and the two repaired frame blobs, verified identical in707a92c6.
Source-only acceptance does not implement the pending root hooks or real inference.

Exclusive NEW paths once activated:

- apps/shared/src/androidMain/kotlin/com/visualworkbench/shared/assistedselection/backend/**
- apps/shared/src/androidDeviceTest/kotlin/com/visualworkbench/shared/assistedselection/backend/**
- Dedicated platform-neutral backend admission/tensor-layout tests under
  apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/assistedselection/backend/**
- NEW core/crates/vw-ffi/src/assisted_source.rs and assisted_source/**,
  tests/assisted_source.rs; NEW shared commonMain/AssistedSourcePixelsApi.kt,
  jvmMain/NativeAssistedSource.kt and dedicated assisted-source tests.
- tools/assisted-selection/{README.md,model-runtime-candidate.json,verify_artifacts.py,convert_mobile_sam.py}
  and synthetic test fixtures under tools/assisted-selection/tests/**.

Do not modify existing AssistedSelectionApi/Interaction, MaskApi, NativeMasks,
selection/import/frame leaves, platform panels, controllers, manifests, Gradle/
Cargo roots, dependency locks, protocol/schema, licensing inventories or any
giant/provider/Paint file. Root keeps final factory/caller/dependency/license
registration. The new assisted_source/source-pixel leaves above are an explicit
grant for the real native reader: use current project workers/bindings/cancellation,
verify actual source hashes and full oriented dimensions, bound every decode/ABI/
foreign copy against the existing256MiB pool, and preserve delivered-byte ownership
through actual retirement. Prefer existing trusted readers when sufficient; no
second decoder, memory pool or unchecked caller pixels. Return root lib/module,
dependency/license and factory/controller registration hunks promptly.
  For JVM native acquisition, reuse root's existing internal
  `withNativeGiantProject(project, suspend (ProjectSession) -> T): T` project
  lifetime borrow from giant candidate79d7d7fc: it holds the actual project's
  close mutex through joined work, including ordinary projects. Map its typed
  Closed/Invalid failure into the selection domain. Do not copy an unpinned
  nativeProjectHandle across suspension or create another close/lifetime gate.
  The helper composes with the root giant candidate; its final-ABI caller test
  remains deferred, so report that source dependency explicitly.
Do not duplicate the mask importer, correction workflow, memory pool or encoded
image/session owner. Parent may reuse the completed offline owner for this lane;
do not create another worker without assignment.

## Exact stable interface

Implement the existing AssistedSelectionBackend.admit(pins) returning
AssistedSelectionRuntime. The five immutable AssistedSelectionPins fields are
modelArtifactSha256, runtimeArtifactSha256, modelLicenseAdmissionId,
runtimeLicenseAdmissionId and runtimeAdmissionId. Match actual locally hashed
artifacts against a closed reviewed catalog; echoing supplied strings is not
admission. No unpinned download, arbitrary executable/model path or approval bool.

admit performs bounded artifact verification/capability inspection only. No model
load, source decode, embedding or scratch allocation until the existing owner
reserves runtime.workingSetBytes and calls encode(source, execution). Declare only
executions actually implemented: Npu, Gpu and/or Cpu. A real CPU path is required;
delegate unavailability must settle all partial owners before an Unsupported
initialization can allow the existing owner's single CPU fallback. Never retry a
prompt. Model/decoder/tensor residency, native/foreign copies, retained embeddings
and scratch all contribute to a conservative finite working-set bound.

encode consumes exact AssistedSelectionSource: SelectionDocument plus page and
attachmentEpoch, including full SelectionBinding/source hash/revision and oriented
dimensions. Capture the existing actual project/currentness provider in the
Android backend factory; do not introduce an unchecked pixels-to-source factory.
Read through the existing trusted native project capability and verify source
identity/page/revision/dimensions before AND after acquisition. Refuse scaled
display proxies, unsupported source/page kinds or over-budget images. Explicit
untagged/color choices stay with the caller; never mutate original bytes/profile.
If the existing source reader is insufficient, return the smallest exact root
hook rather than substitute a thumbnail or arbitrary ByteArray.

The returned AssistedSelectionSession owns one real encoding and reports its
exact source/execution. propose(request) reuses embeddings, honors positive/
negative points and box, returns exclusive full-oriented-document 8-bit coverage
with exact source/sequence/dimensions, and joins actual inference on cancellation.
close returns only after native interpreter/delegate/buffers/workers retire.
Failed partial encode/close must remain reachable from the runtime/session so the
existing interaction retains its working-set charge; no GC/timeout-as-retirement.
Use the current bounded prompt/proposal/import/correction workflow unchanged.

## Model/runtime admission and results

The portable research record proposes MobileSAM vit_t at source commit
f706ad9c4eb7f219c00d9050e46328518ffb65d2; source checkpoint40,728,226bytes,
SHA2566dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f,
and LiteRT2.3.0 CPU/GPU artifacts. Licenses/POM/artifact hashes are recorded in
docs/evidence/SEL001_BACKEND_ADMISSION_RESEARCH_20261018.json at9e41ed1d.
These are research candidates, not closed production admissions. Deployable
encoder.tflite/prompt_decoder.tflite conversion, their exact hashes/tensor
contracts and transitive/native notices remain OPEN. Implement the real converter,
loader/encoder/prompt decoder, not fake coverage. Never use the .pt source hash as
a converted model hash. Use a deterministic closed manifest identity for both
converted artifacts and another for actual runtime children, verifying every
child's bytes/hash; record this lowering without changing the shared five fields.
Do not enable unsafe pickle or Flex/custom/vendor dependencies as a shortcut.
Inspect primary official sources and existing policies
before proposing a dependency. Public free artifacts may be downloaded into an
owned temporary directory for inspection/hash/license evidence; no user images,
paid service, key, device/driver install or live provider call. Do not commit a
large binary or enable bundling without the reviewed redistribution/license/
artifact pins and root policy registration. Missing or rejected artifacts must
remain visibly Unsupported/Unavailable, while independent source work continues.

Return exact model/runtime names/versions/official URLs/licenses/SHA-256/size,
input/output tensor contracts, orientation/prompt/coverage transforms, CPU and
delegate capability evidence, peak-memory estimate and minimal root hooks. Record
actual cheap admission/layout/unit checks separately from real inference. Real
synthetic-image inference may run only under parent resource allocation with
admitted artifacts; S23 NPU/GPU/latency/quality/manual-correction journeys remain
in the final deferred ledger. No <=100ms or accurate-segmentation claim from a
stub, compile, metadata flag or unrun test.
