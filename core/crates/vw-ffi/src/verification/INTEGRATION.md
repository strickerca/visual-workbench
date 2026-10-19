# T5.01 verification integration handoff

## Framed verification (2026-10-19 V leaf)

After composing root's `vw_package::verification_image_geometry` at
`6fd99bf7f7523071f0f955603461c42af387312c`, retain this branch's new
`verification/geometry.rs`, DTOs and shared `VerificationApi.kt` /
`NativeVerification.kt` leaves. Regenerate the root-owned UniFFI bindings for
`PackageCatalog.verification_coordinate_pair(pair, cancellation)` and its
`VerificationCoordinatePair` / `VerificationImageCoordinates` records. The
shared `verificationCoordinates(catalog, pair)` adapter calls that exact method;
Desktop comparison now calls it. The native method reopens both exact checked
catalog packages, checks target profiles, projects their canonical geometry,
includes binding and source-asset identities, and rechecks the catalog. It
returns continuous `content_bounds` separately from rounded-up `content_size`
so a fractional edge pixel is not treated as fully supported registration
content. It grants no capture, receipt, alignment or review authority.

`McpNormalizedVerification.kt` maps retained inbox tiles through those exact
matrices onto a common D-space grid. Every sampled physical pixel must have
its full square inside continuous content, and each tile/receipt/canvas identity
is checked. Reads are limited to 16 tiles per side and 512² output samples;
unsupported support or ambiguous registration stays unresolved. Semantic IDs
can supply an integer D shift; the existing bounded image registrar also runs
on normalized D tiles. Framed per-marker crops use that same grid, including
original-only, after-only and equal-size quarter turns. Failure compilation
rechecks paired pixel availability; native `prepare.rs` checks whole original
and translated failed-marker regions against saved crops before host work.
Native pair/source currentness uses checked geometry rather than the temporary
frame refusal. The ordinary scalar path is retained for unframed packages.

Cheap isolated Kotlin compilation and execution passed five normalized-pixel
regression methods plus the complete synthetic workflow routing probe (including
framed semantic comparison/failure routing, rotated after-frame image
registration, partial-crop refusal and historical source mismatch). Rustfmt
and whitespace checks passed. Native FFI/package
tests, generated-binding compilation, full Desktop/ABI and real capture/device
acceptance are **unrun**: the parent holds the shared heavy slot. Exact deferred
checks after root composition: `cargo test --locked --offline -p vw-ffi verification -j2`,
`cargo test --locked --offline -p vw-package verification_geometry:: -j2`, and
the focused Desktop verification tests. No OS recapture, actual target
registration or A30 pass is claimed.

Base `47f2b82da1799a8481af7e92998f5ea56dc9c087`; branch
`cloud/t5-01-history-20261018`; grant
`docs/ROADMAP_ROOT_HOOK_GRANTS_20261018.md`, V extension. Before editing, compared
actual main `6d505416` and this exact publication against the prior V candidate
`490de501`: all previously owned V files were byte-identical, including the
accepted VF1 256 MiB default. No recovered archive was overlaid.

## Result and ownership

The existing private store now enumerates saved reviews through native/shared
owners. Desktop settings load the current original's history, paginate it, and
reopen a selected review against both checked packages and the exact retained
before/after inbox PNGs. Saved notes and per-marker comparisons can display after
restart even when the original's document revision has changed. Saving decisions
and compiling failures still use exact source/CAS/currentness checks. Historical
selection never restores a capture selector or grant. No observed app restart,
OS capture or product gate is claimed.

This extension changes only the two granted `vw-store` verification_review files,
V-owned `vw-ffi/src/verification*` and its dedicated integration test, the five
shared Verification API/adapter/test files, Desktop MCP files and dedicated tests,
and this handoff. Root Cargo/locks, ABI generation, registration, NativeCore,
controllers/Main/WorkbenchApp, capture delivery and archive.rs remain untouched.
No Node changes were needed: the prior two bounded tools already cover T5.01.

Store history uses its existing locked connection/deferred transaction, validates
the <=64-row namespace metadata before raw reads, validates row/schema/key/pair/
project binding before note-vector decoding, and admits <=8 records / <=2 MiB
encoded bytes. Cursor is exactly `vw.verification-review.v1.` plus64 lowercase
hex digits, exclusive lexicographic order; next_cursor is the last returned key
only when another row remains. Byte-cap pages advance without fetching raw
lookahead. Fresh version-zero reads never persist or appear in history.

Missing after-source leases recover only from the capture engine's existing
app-private `mcp-captures-v1` inventory: <=32 direct owned workdirs, exact bounded
owner marker, canonical nonsymlink ancestry, sequential existing core.open,
exact project/document/revision comparison, joined NonCancellable close before
return/next candidate. Engine reopens and rechecks; native operations additionally
validate source asset/capture/blob/compiler identity. There is no new index,
source owner, database, selector persistence or cleanup.

The 2026-10-19 P2 repair retains every exact opened capture project in the
engine until its `close()` succeeds. A failed recovery/verification/compile
close blocks the next `core.open`; shutdown retries the same owner through the
coordinator, service lifetime and bundle pin. This changes no native owner API.
Focused tests cover first-candidate failure, a later borrow, two-source cleanup,
coordinator retry and sealed service restart. The production native capture
adapter and actual process teardown remain unrun.
The exact root-owned `NativeProject.close` retry is meaningful for failures in
`drainUnadoptedGiants()` before `closeSession`/`destroy`. If `closeSession` or
`destroy` itself throws, that wrapper currently sets `closed` in `finally`; a
later `close()` can return without retrying native retirement. Root should review
that separate native-owner behavior before claiming all close errors recover.

The history admission query now uses two indexed `settings` primary-key ranges
for TEXT and BLOB namespace keys. The source-SQL SQLite probe includes malformed
BLOB/TEXT keys and 40,000 unrelated rows: empty history costs 46 VM steps on
SQLite 3.53.1 (40 on system 3.46.1), while the old predicate costs 200,015.
The 65-row limit sentinel remains bounded at 1,196/1,190 steps. Native store
tests remain unrun pending the shared heavy slot.

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

## Additive ABI and shared contracts

Existing verification records/methods remain as in VerificationApi.kt and
verification.rs. New native record and async ProjectSession methods:

```text
VerificationReviewHistoryPage { snapshots: Vec<VerificationReviewSnapshot>, next_cursor: Option<String> }
verification_review_history(after: Option<String>, limit: u32, cancellation: Arc<Cancellation>) -> PackageResult<VerificationReviewHistoryPage>
check_verification_history(expected: VerificationReviewSnapshot, cancellation: Arc<Cancellation>) -> PackageResult<()>
```

The second method compares the exact durable snapshot in the borrowed original
store. It rejects v0, stale snapshots and foreign projects; it does not assert
historical document currentness or open an after source. Native adapters keep
existing closure/cancellation worker checks. Shared `verificationHistory(original)`
returns borrowed `WorkbenchVerificationHistory.page(after=null,limit=8u)` and
`.check(expected)`. It validates decoded shape/owner/cursor, defensively copies
both lists, redacts private diagnostics and joins canceled reads through existing
ownPackageResult. Native storage alone measures encoded admission bytes.

VF1 remains fixed: compileFailures defaults to256 MiB, the native ceiling;
explicit16..256 MiB admission is unchanged. Unix timestamp units remain ms.
Failure packages keep the exact six admitted lineage fields and after-source
project identity. Manual notes are absent from portable output/agent wire.

## Remaining root hooks

1. Register `mod verification; pub use verification::*;` in FFI lib.rs, apply the
   three exact native owner helpers above, then regenerate the core ABI once.
2. Apply the two exact existing-owner helper hunks documented at the end of
   shared jvmMain NativeVerification.kt: `ownNativeCompiled(NCompiled)` and
   checked `nativeMcpInbox(WorkbenchMcpInbox)`. No second owner/lifetime.
3. Root mcp/src/server.mjs imports `registerVerificationTools` from
   `./verification.mjs`, then after its private tool helper calls
   `registerVerificationTools(tool, { packages, owner, principal });`.
   Tool census6->8; no agent verdict/note/translation setter.
4. Capture delivery, while it still owns the real target, receipt, opened project
   and document, builds `NativeMcpCaptureEngine.editorOrigin(target, receipt,
   opened.info(), document)` and calls the already-started service owner's
   `rememberEditorCapture(origin)` after successful editor delivery. An optional
   callback failure preserves that transferred editor owner and reports unavailable
   verification. Do not reconstruct an origin from portable metadata or a title.
5. Main installs the existing editor capability on each replacement MCP owner:

```kotlin
owner.attachVerificationOriginal(object : McpVerificationOriginalAccess {
    override fun owns(expected: PackageInfo) = controller.ownsVerificationOriginal(expected)
    override suspend fun <T> withOriginal(expected: PackageInfo,
        body: suspend (WorkbenchProject) -> T): T =
        controller.withVerificationOriginal(expected, body)
    override suspend fun <T> withCurrentVerificationOriginal(
        body: suspend (WorkbenchProject) -> T): T =
        controller.withCurrentVerificationOriginal(body)
})
```

The additive interface signatures are exactly the three overrides above. Both
borrow helpers hold EditorController's existing edits mutex throughout body and
reuse compileForMcp's closing/busy/draft predicates. `withOriginal` additionally
checks actual project/document/hostSeq/stateHash against expected.binding;
`withCurrentVerificationOriginal` lends the current original before a pair is
known. Neither closes/replaces it. False identity selection may use retained
engine originals; an actual stale matching editor failure never falls back.
Join MCP shutdown before editor/native close. No excluded root hooks were edited.
The default current-original interface body refuses until root supplies it.

## Archive portability finding for root

archive.rs currently uses `key GLOB prefix*` for both review export scrubbing and
import refusal. This matches BLOB keys only with some SQLite compile options.
Cheap actual-SQL fixtures: system SQLite3.46.1 has LIKE_DOESNT_MATCH_BLOBS and
misses a BLOB-prefix row; Python SQLite3.53.1 lacks that flag and matches it. The
first probe assumed identical compile options and failed; the separate runtimes
resolved it. Cached libsqlite3-sys0.38.2 bundled defaults do not set the flag, so a
production archive leak is **not demonstrated**. Raw namespace admission should
nevertheless use one build-independent predicate, preserving malformed-note
scrubbing and imported-private-row refusal:

```rust
// Export DELETE and import SELECT EXISTS, respectively:
"DELETE FROM settings WHERE substr(CAST(key AS BLOB),1,?1)=CAST(?2 AS BLOB)"
"SELECT EXISTS(SELECT 1 FROM settings WHERE substr(CAST(key AS BLOB),1,?1)=CAST(?2 AS BLOB))"
rusqlite::params![crate::VERIFICATION_REVIEW_PREFIX.len() as i64,
                 crate::VERIFICATION_REVIEW_PREFIX]
```

Both predicates passed the system SQLite synthetic BLOB fixture while preserving
an unrelated setting. archive.rs is outside this grant; root owns that repair and
actual export/import BLOB regressions. Existing normal privacy test now also
checks actual export/import history is empty; it is authored, unrun on this tree.

## Checks and proposed central deferred-ledger rows

Executed: cached Kotlin2.4 -Werror current-source probe **87 cases**: shared14,
JVM ownership2, workflow27, coordinator7, registration13, source recovery18,
service lifetime6. Exact shared API/current production workflow/coordinator/
recovery/lifetime leaves compile; only JNI factories are substituted. JUnit bodies use
small assertion facades; Linux symlink fixtures cannot silently skip. Temporary
classes/declarations disposed. This does not compile NativeVerification, native
capture adapter, generated ABI, Compose or the assembled app.

VF1 negative control on the earlier history tree restored only the temporary shared128 MiB
default: the actual nonempty Desktop caller failed as expected with Package:Limit,
preserving the prior accepted regression. Pinned Rust1.99 rustfmt parse/format and
diff whitespace guards pass on this follow-up. Native Cargo/Gradle/ABI/device/package
work did not run; one heavy slot remains parent-owned. Earlier accepted Node39/39
and foundation native receipts remain bound to their earlier source only.

Retained cloud text receipts (no captures):
`/tmp/vw-t5-01-history-focused.log` SHA256
`20e02a6430b1026be66d50d1bf4ebd82782cc2e25fc856092b90b58e75276eba`;
`/tmp/vw-t5-01-history-budget-negative-control.log` SHA256
`38d6096c1c41168531df87ba845b8d10cdcd8ae066d75014b9c82c0711790e95`.
Follow-up Kotlin receipt `/tmp/vw-t5-01-history-p2-o-kotlin.log` SHA256
`534a0545ca117a4ea0e6a71db49f1a564c9deb0c8c00a5d320e7cfff98815108`.
SQL probe: `python3 core/crates/vw-store/tests/probe_verification_history_sql.py`
and its `--legacy-scan-negative-control` mode, both Python SQLite runtimes.

| Scope | Exact check/journey | Prerequisite and expected result | Status |
|---|---|---|---|
| Private store history | `cargo test --locked -p vw-store --test verification_review` | Parent slot;14 tests including6 new history cases: reopen64 rows/all page sizes, v0 absence, cursor/2 MiB/corruption/key/project limits; unchanged portable privacy | Authored, unrun |
| Native verification | `cargo test --locked -p vw-ffi verification`; `cargo test --locked -p vw-ffi --test verification`; clippy all targets with-D warnings | Native helper/registration hooks; exact private history check/reopen and existing real failure compiler journey | Authored, unrun |
| Shared/Desktop assembly | `bash apps/gradlew -p apps :shared:desktopTest --tests '*Verification*' :desktop:test --tests 'com.visualworkbench.desktop.mcp.*' -Pkotlin.incremental=false --no-daemon --no-parallel --max-workers=2` | Regenerated ABI, all root hooks and parent slot; real MPP/JUnit/Compose/native capture integration | Deferred;87 focused cases passed |
| Registered MCP | `node --test test/*.test.mjs` from mcp with pinned Node24 | Two-line root registration/tool census8 | Prior leaf39/39 retained; full server unrun |
| Restart/history/privacy | Save review -> full app/service restart -> open original -> Load/Open saved review -> compare -> fresh target/grant -> exact-source save/failure compile; export/import absent private history including BLOB prefix | Root current-original/capture hooks, actual Windows app and archive predicate repair | Source path implemented; actual runtime/privacy journey unrun |
| A30 / VERIFY-001..004 / AGENT-006 | Original mark/publish -> granted same-target recapture -> per-marker review -> failures package -> text/PNG Compare; revoke/disconnect/retire during work | Authorized real target/runtime/agent transport, coherent parent tree; no inferred pass/fail | Product/hardware acceptance unrun |

Repeatable cheap command:
`python3 apps/desktop/src/test/kotlin/com/visualworkbench/desktop/mcp/probe_verification_workflow.py`
(add `--legacy-failure-budget-negative-control` for expected failure). Root alone
updates the central ledger. No main merge/release/install/provider/credential or
device/security action occurred. Krita and unperformed physical/G0-G5 gates remain
deferred. Next task is root integration plus narrow source review, then the single
coherent validation sequence; T5.01 remains L, owner-selected Sol/xhigh applies.
