# T5.01 verification caller handoff

Base: accepted `6d505416963fb42009921bd3fb8639994e73752a`; grant:
`1dc90e2:docs/ROADMAP_PARALLEL_IMPLEMENTATION_GRANTS_20261018.md`, V lane.
Candidate branch: `cloud/t5-01-workflow-20261018`. Actual remote main was
checked at `6d505416` again before closure. Recovered `0d91b705` lacks the admitted
three Desktop leaves and owner guards; none of that older source was overlaid.

Implemented source: Desktop same-target/profile recapture, checked after PNG
comparison receipts, manual private CAS review with exact-ID retry, bounded
semantic/unique protected-pixel alignment, paired crops and failure-only native
compilation; two bounded agent tools. Existing untrusted `submit_result` Compare
remains, with its UUIDv4 receipt bug corrected to the existing core UUIDv7 API.
No product gate or real capture outcome is claimed. These reserved hooks remain
unapplied; source formatting/focused Kotlin checks do not compile the native ABI.

## Registration

In `core/crates/vw-ffi/src/lib.rs`, add:

```rust
mod verification;
pub use verification::*;
```

The admitted store/package leaves, archive private-copy review removal and
six-field package lineage are already activated on the accepted base. Do not
duplicate them or alter the portable schema version.

## Existing compiled owner adoption

Inside the existing non-exported `impl CompiledPackage` in
`core/crates/vw-ffi/src/packages/compile.rs`, add this capability constructor.
It acquires the existing two-owner `Permit` before full original, clone or
compiler work; all-pass needs no permit. Error/cancel/future-drop destroys the
same permit. The closure
adopts one ordinary checked `Package`, with existing describe/dispose semantics.

```rust
pub(crate) fn verification_builder()
    -> PackageResult<impl FnOnce(vw_package::Package) -> Arc<Self> + Send + 'static>
{
    let permit = Permit::acquire()?;
    Ok(move |package| Arc::new(Self {
        prepared: Mutex::new(Some(Arc::new(Prepared {
            package,
            _permit: permit,
        }))),
    }))
}
```

## Existing full original resolver reuse

In `core/crates/vw-ffi/src/packages/compile.rs`, next to `originals`, add:

```rust
pub(crate) fn verification_originals(
    state: &ProjectState,
    document: &Id,
    budget: u64,
    closed: &AtomicBool,
    cancellation: &Cancellation,
) -> PackageResult<BTreeMap<AssetId, Vec<u8>>> {
    originals(state, document, budget, &Stop { closed, cancel: cancellation })
}
```

In `core/crates/vw-ffi/src/packages.rs`, add:

```rust
pub(crate) use compile::verification_originals;
```

No new filesystem resolver, decoder, transaction or source ownership is added.
The existing immutable blob reads verify size, regular-file identity and BLAKE3.

## Existing checked catalog owner read

Add a separate **non-exported** `impl PackageCatalog` in
`core/crates/vw-ffi/src/packages/catalog.rs`:

```rust
impl PackageCatalog {
    pub(crate) async fn verification_package(
        &self,
        package_id: String,
        target: String,
        manifest_sha256: String,
        cancellation: Arc<Cancellation>,
    ) -> PackageResult<vw_package::Package> {
        let k = key(&package_id, &target)?;
        hash(&manifest_sha256)?;
        let closed = self.closed.clone();
        self.worker.call(move |state| {
            Ok((|| {
                check(&closed, &cancellation)?;
                state.guard()?;
                let entry = state.entries.get(&k).ok_or(PackageError::Stale)?;
                if entry.manifest_sha256 != manifest_sha256 {
                    return Err(PackageError::Identity);
                }
                let package = files::package(&state.root.join(&k), &cancellation)?;
                let info = dto::describe(&package)?;
                if info.package_id != entry.package_id
                    || info.target != entry.target
                    || info.manifest_sha256 != entry.manifest_sha256
                {
                    return Err(PackageError::Integrity);
                }
                check(&closed, &cancellation)?;
                Ok(package)
            })())
        }).await?
    }
}
```

This returns the existing `files::package` checked owner, preserving guard,
retirement, quota/path/hash/PNG admission. The caller never supplies a filesystem
path. It avoids repeatedly decoding every package image for each `read_file`.

## Additive ABI

Records:

```text
VerificationPackageKey(package_id:String,target:String,manifest_sha256:String)
VerificationPair(original:Key,after:Key,receipt_id:String,receipt_blake3:String)
VerificationVerdict = Pending | Pass | Fail
VerificationMarkerReview(original_number:u32,verdict:Verdict,note:String)
VerificationReviewSnapshot(pair:Pair,project_id:String,marker_count:u32,version:u64,markers:Vec<Review>)
VerificationMarkerTranslation(original_number:u32,dx:f64,dy:f64)
```

Three exported `ProjectSession` methods (`self` is the original retained source
project), each async and returning `PackageResult`:

```text
verification_review(after:Arc<ProjectSession>,catalog:Arc<PackageCatalog>,inbox:Arc<McpResultInbox>,pair:VerificationPair,cancellation:Arc<Cancellation>) -> VerificationReviewSnapshot
change_verification_review(after,catalog,inbox,pair,expected_version:u64,operation_id:String,original_number:u32,verdict:VerificationVerdict,note:String,cancellation) -> VerificationReviewSnapshot
compile_verification_failures(after,catalog,inbox,pair,expected:VerificationReviewSnapshot,translations:Vec<VerificationMarkerTranslation>,package_id:String,created_at_ms:i64,memory_budget_bytes:u64,cancellation) -> Option<Arc<CompiledPackage>>
```

Timestamp units are Unix milliseconds, range 0..253402300799999. Caller budget
is 16..256 MiB; real compiler/source working sets can refuse within that range.
Native package admission remains 64 markers, 32 MiB total, 4 MiB per derivative.
Store reviews remain bounded to 512 markers, 32 KiB UTF-8 per note and 128 KiB
total notes. UUIDs including receipts are canonical UUIDv7, matching the actual
native inbox and admitted store; Desktop `UUID.randomUUID()` receipts have been
replaced by the existing core UUIDv7 generator in this candidate.

`project_id` on the snapshot belongs to the original review store. A failure
package belongs to the exact **after** source project, which may have a different
project/document identity. Shared validation must not equate those project IDs.

## Authority/currentness limits

Native checks exact catalog package/receipt/PNG bytes and both real source
workers before and after work. The original store worker fences compilation on
actual CAS snapshot/source/cancel/closed predicates; no constant-true callback.
Normal full-original compilation uses an isolated existing HostSequencer and
DocumentView, then the admitted failure compiler. It changes neither live source.
Manual notes remain private; exactly six admitted lineage fields are preserved.

The trusted Desktop capture ticket/grant/indicator owner establishes causal
recapture association. Imported lineage or an agent PNG cannot establish it.
Both agent tools recheck the original's private selector and live grant. Only
manual UI actions can change verdicts/notes; exact pending operation IDs survive
uncertain replies. Paired images refuse ambiguous/low-texture/unresolved,
different-scale/subpixel-phase mappings. Native validates owner-supplied finite
translations and full-source bounds; it does not independently authenticate the
Desktop alignment algorithm. Parent must preserve joined source/retirement
ownership through native calls and final publication.

## Shared adapters and Desktop root hooks (unapplied)

1. Apply exactly the two helper hunks at the end of new
   `apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeVerification.kt`:
   `ownNativeCompiled(NCompiled)` wraps existing `NativeCompiled` and disposes a
   failed construction; `nativeMcpInbox(value)` checks and borrows existing inbox.
   Do not alter PackagesApi/NativeCore or invent a compiled owner.
2. Register the native module above, then perform the parent's one core binding
   generation. New shared methods and DTO signatures are in `VerificationApi.kt`.
3. Add these two lines to root-owned `mcp/src/server.mjs`: import
   `registerVerificationTools` from `./verification.mjs`; immediately after its
   private `tool` helper call
   `registerVerificationTools(tool, { packages, owner, principal });`.
   Protocol tool census changes from6 to8. No agent verdict/translation setter.
   Owned Desktop Host/Service already forward `verification_recapture` and
   `verification_compare` using the exact fields in the new Node leaf.
4. The existing Desktop capture owner must deliver the real origin while it has
   the exact native `target`, `receipt`, newly opened `project` and `document`:

   ```kotlin
   val info = opened.info()
   val origin = NativeMcpCaptureEngine.editorOrigin(target, receipt, info, document)
   // Only the already explicitly started service; do not start it automatically.
   mcp.state.value.owner?.rememberEditorCapture(origin)
   ```

   Add a callback from `DesktopCaptureController`'s successful delivery rather
   than reconstructing this from a manifest/title. It currently routes only
   `(opened,destination,warning)`, so this callback is a real reserved capture
   seam. Retain the origin after successful destination delivery, even if
   semantic collection was unavailable. Failure of this optional MCP callback
   must preserve the already delivered project/transfer and report verification
   unavailable; it must not close that transferred editor owner. Main may need
   to initialize its `mcp`
   lifetime before the capture controller to pass the callback. The native
   adapter refreshes exact HWND/PID/process-created identity, then returns its
   existing private selector. Coordinator records source→selector association;
   later `publish` binds marked revisions. No capture grant is created here.
5. On every non-null/replaced `mcpState.owner`, Main installs the borrowed editor
   capability (new owned `McpVerificationOriginalAccess`):

   ```kotlin
   owner.attachVerificationOriginal(object : McpVerificationOriginalAccess {
       override fun owns(expected: PackageInfo) = controller.ownsVerificationOriginal(expected)
       override suspend fun <T> withOriginal(expected: PackageInfo,
           body: suspend (WorkbenchProject) -> T): T =
           controller.withVerificationOriginal(expected, body)
   })
   ```

   Add these two small methods in reserved `EditorController`, using its existing
   `mutable`, `project`, `generation`, `closing`, `edits` and busy/draft predicates:
   `ownsVerificationOriginal` selects only the current project/document identity;
   `withVerificationOriginal` holds `edits.withLock` throughout body, rejects
   closing/busy/unsaved gestures/instructions, compares actual info project ID,
   document, hostSeq and stateHash to expected.binding, and borrows that exact
   `WorkbenchProject`. Reuse `compileForMcp`'s predicates; do not close/replace it
   or add another lifetime. False ownership uses retained engine sources; an
   actual stale failure never falls back. MCP shutdown must join before editor
   close/native runtime release, preserving existing Main retirement order.

The new Compare/settings UI callsites are already connected inside owned MCP
files; their existing Main signatures stay compatible. Without hooks4–5, ordinary
editor-origin packages conservatively lack recapture/source authority. The
engine's own capture-source leases work independently; they retain full private
originals, open through existing core API and join before closing.

Admitted store reviews persist across close/reopen and export strips private
rows; the new FFI exact-pair reopen journey is authored but unrun. Desktop pair routing and target/grants are session
only. This candidate does **not** restore a historical verification pair in the
Desktop UI after service/app restart; no selector/grant is persisted. A future
history route requires admitted private pair enumeration/reassociation from the
existing source owner, rather than inferring authorization from imported lineage
or an agent PNG. That UI clause remains open; the native reopen test is separate.

## VF1 budget correction and history grant request

VF1 review of16afe4e1 found the shared128MiB default cannot fund any nonempty
failure follow-up. Shared `WorkbenchVerification.compileFailures` now defaults
to256MiB, exactly the existing native ceiling; explicit caller budgets and native
16..256MiB admission remain unchanged. Call-chain audit:
`McpVerificationPanel` → `DesktopMcpCoordinator.compileVerificationFailures` →
`McpVerificationWorkflow.compile` → shared default → `NativeVerification` →
`ProjectSession::compile_verification_failures`. Neither Desktop caller nor JVM
adapter replaces the budget. Native fixtures still explicitly use256MiB and
are **not** the regression for this fix.

The focused probe now calls the real Desktop workflow with a nonempty failure,
no budget argument/override, and real semantic translation. Its injected backend
observes256MiB, returns a synthetic follow-up owner, and checks the exact after
binding/failed marker mapping. This proves default forwarding, not native codec
execution. Negative-control mode changes only the temporary shared default back
to128MiB; the same actual caller fails the compiler-reserve guard with Limit.

Historical-pair retrieval needs an extra grant for precisely:
`core/crates/vw-store/src/verification_review.rs` and
`core/crates/vw-store/tests/verification_review.rs` (existing foundation is
read-only to this lane). Proposed additive API on the existing locked store:
`verification_review_history(after: Option<String>, limit: u32) ->
VerificationReviewResult<VerificationReviewHistoryPage>` with
`VerificationReviewHistoryPage { snapshots: Vec<VerificationReviewSnapshot>,
next_cursor: Option<String> }`. Cap limit at8, aggregate admitted bytes at2MiB;
use canonical namespace/hash cursors, schema/row-size/key/project/record checks
before allocating/returning, and the existing settings connection/transaction.
No new DB/index, selector persistence, portable lineage authority or second
catalog. Existing native/shared verification leaves and Desktop controls can
consume that private page inside their current grant; only manually saved
records exist today (fresh v0 reads do not persist a history row). Tests must
cover reopen/pagination/corruption/bounds and unchanged portable privacy.

The reserved editor/Main source hook also needs a narrow parent-applied way to
borrow the **currently opened original** under the existing `edits` mutex before
its historical pair is known. Add a `withCurrentVerificationOriginal(body)`
overload alongside `withVerificationOriginal(expected,body)` and supply it via
`McpVerificationOriginalAccess`; no raw paths or independent project lifetime.
All later modifications/compiles retain exact pair/source currentness checks;
history selection must not restore capture selectors or grants. The two store
paths above remain unmodified pending that explicit grant.

## Focused checks and proposed deferred-ledger rows

Preserved prior unchanged Node24 bounds/packages/verification39/39 evidence;
fresh VF1 correction checks:
pinned Kotlin2.4 `-Werror` focused current coordinator/workflow/shared API,
manual-review/late-result, six existing recapture regressions and routing
regressions (31 cases); registration leaf13/13 via
temporary standalone JUnit-body facade; pinned Rust1.99 rustfmt parse/format of
new leaves; commonMain import/whitespace guards. Temporary declarations/classes
were disposed. The Kotlin probe deliberately substitutes the JNI factory only;
it injects synthetic checked interfaces and does not claim native/Compose/ABI or
OS capture acceptance. No broad build, native test, generation, provider/device
call, installation, packaging or main merge ran.

| Candidate scope | Exact command/journey | Prerequisite | Expected outcome | Status |
|---|---|---|---|---|
| Native CAS/compile | `cargo test --locked -p vw-ffi verification`; `cargo test --locked -p vw-ffi --test verification`; `cargo clippy --locked -p vw-ffi --all-targets -- -D warnings` | Three native helper hunks/module registration; parent shared heavy slot and normal build.ps1/license entrypoint | Four new unit cases and one integration journey pass: pending/stale refusal, real after originals, exact CAS retry/reopen, private notes, six lineage fields, unchanged live state | Authored, unrun |
| Existing foundation | `cargo test --locked -p vw-store --test verification_review`; `cargo test --locked -p vw-package --test verification` | Composed tree, parent slot | Admitted CAS/privacy/failure compiler unchanged | Earlier foundation evidence retained; this tree unrun |
| Shared/Desktop/Android | `bash apps/gradlew -p apps :shared:desktopTest --tests '*Verification*' :desktop:test --tests 'com.visualworkbench.desktop.mcp.*' :android:testDebugUnitTest -Pkotlin.incremental=false --no-daemon --no-parallel --max-workers=2` | One regenerated core ABI, helper/root hooks, existing runtime inputs; parent aggregate slot | Actual JUnit/MPP/Compose/capture adapter compatibility; existing lifecycle/retirement tests and new tests pass | Deferred aggregate; focused Kotlin subsets executed |
| MCP root registration | `node --test test/*.test.mjs` from mcp with pinned Node24 | Two-line registration and tool census update6→8 | Both protocol eras expose exact eight tools; grants, cancellation, checked identities, bounded crops | New leaf suite39/39; full registered server unrun |
| A30/VERIFY-001–004/AGENT-006 | Capture original→mark→publish→explicit grant→agent recapture→per-marker aligned review→fail one→compile follow-up→submit text/PNG→Compare; revoke/disconnect/retire during work | Reserved capture/editor hooks, authorized Windows target/runtime, actual agent transport, parent runtime slot | Exact original/after identity and marker lineage, allpass null, pending/stale refusal, notes private, no implicit acceptance | Unrun hardware/product acceptance |
| Restart/history/privacy | Save review→source close/reopen exact pair; portable export/import→private-row absence; separate full app restart→manual history retrieval | Registered native API; original source owner; additional admitted Desktop history route for last step | Notes survive locally, never portable; targets/grants require fresh explicit association | Native integration regression authored/unrun; Desktop historical-pair UI remains open |

Cheap repeatable current-source probe:
`python3 apps/desktop/src/test/kotlin/com/visualworkbench/desktop/mcp/probe_verification_workflow.py`.
Use the central deferred ledger only in the coordinator's tree. Krita, phone
screen recapture, S Pen/performance and all unperformed G0–G5/A30 gates stay open.

VF1 focused regression commands (no native/Gradle build):

```text
python3 apps/desktop/src/test/kotlin/com/visualworkbench/desktop/mcp/probe_verification_workflow.py
python3 apps/desktop/src/test/kotlin/com/visualworkbench/desktop/mcp/probe_verification_workflow.py --legacy-failure-budget-negative-control
```

The first passed44 focused Kotlin cases. The second failed as expected only
after restoring the old default, with the nonempty default caller's `Package: Limit`;
the earlier all-pass currentness case still passed. No native budget was changed.
