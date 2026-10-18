# T5.01 verification loop: dependency handoff, 2026-10-18

**Disposition: no product-source change; existing result Compare is reused.**
Integration owner: `01a14eea-d204-762c-94b8-f07105b5001e`. This lane owns only
this file. No manifests, lockfiles, schemas, generated bindings, FFI registration,
controllers, app roots, giant-image or T3.07/T3.11 leaves were changed.

## Compared sources

Remote `refs/heads/main` was queried and fetched before this decision:
`6f3e2f3b9ed7a760d4dd7f4c6f26fa44dc5ad88d`, exactly the starting checkout.
Read `AGENTS.md`, v2 requirements/specification/plan, the latest roadmap ledger,
M4 checkpoint and giant-producer/following-lane admission. `/workspace/.agents`
is empty; the repository has no `.agents/skills` directory. T5.01's plan
dependency remains T4.09; the resumed software scope does not pass that gate.

Inspected recovery commit `0d91b70558a995fa7654021de7465b1d3f615e3d` and fetched
recovery tip `473395197f6437586a8b0c05fa8cd35aa6800441`, including the manifest
and relevant `transport/t3-stream-finish/after` package, inbox and shared API
postimages. No dedicated T5.01 implementation was found. Recovered
`PackageInfo.captureProvenanceJson` / `Capture.stream_save` adds saved-frame
provenance; it does not supply the package-to-selected-window relation or a
verification alignment/review receipt. No recovered postimage was overlaid.

## Actual coverage and the narrow dependency

| Requirement | Existing implementation / missing behavior |
| --- | --- |
| AGENT-006 | `mcp/src/server.mjs` already implements `submit_result`; `DesktopMcpCoordinator.submitResult` persists an exact package/hash-bound inbox receipt. `McpComparePanel.kt` displays literal text or immutable Before/After regions. Reuse this path; source coverage is not fresh hardware acceptance. |
| VERIFY-001 | `NativeMcpCaptureEngine` already retains `selector -> CaptureTarget` and calls `CaptureService.refreshTarget(selected)` before capture. The native target includes HWND, PID and process creation time. There is no retained relation from the original published package/hash to that selector. Each compile creates a fresh capture/project/package. The public manifest's `Capture` has session/frame metadata, not this private target relation. A caller choosing a selector anew cannot prove it is the original package's target. |
| VERIFY-002 | Package marker bounds, crop mappings and semantic references already exist. Inbox `pixels` reads bounded regions; its Before is the package's compiled clean image. Compare currently takes manual X/Y within the shared extent. There is no registration transform, correspondence receipt or per-marker aligned pair. Equal extents do not establish alignment. Semantic `eid` values are snapshot-prefixed (`vw-semantics/src/snapshot.rs`); matching raw EIDs between fresh snapshots is insufficient. |
| VERIFY-003 | No owner pass/fail persistence or failure-only package operation was found. `vw-package/src/verify.rs::from_compiler` requires contiguous marker numbers `index + 1` and matching image/crop names. Simply deleting passed markers breaks verification; silently renumbering loses original references. `Manifest`/`Extensions` reject unknown fields and currently have no typed follow-up origin mapping. |
| VERIFY-004 | The six registered tools are `list_packages`, `get_package`, `get_marker`, `get_semantic`, `capture_window`, `submit_result`. `McpOwnerActions` and the host owner/request dispatcher support capture/submission only. There is no recapture/comparison callback to register. Preserve `Grants.admit`, the exact principal/selector boundary, indicator acknowledgement, cancellation settlement and publication drain. |

The immediate blocker is the missing **private original-package-to-target and
review receipt contract**, not missing hardware evidence. Implementing a second
capture service, accepting an agent-supplied alignment boolean, or inventing a
parallel failures format would bypass the existing owners. This lane therefore
returns a dependency finding rather than unregistered speculative scaffolding.

## Exact integration hooks needed to resume

1. Integration must retain the exact published `(package ID, target, manifest
   SHA-256, WorkflowBinding, source asset)` to the existing private selector and
   selected `CaptureTarget`, with retirement/service-close behavior defined.
   Reuse `NativeMcpCaptureEngine.compile`, `CaptureService.refreshTarget` and
   `DesktopMcpCoordinator.capture`'s grant/indicator/owned-work path. Do not put
   HWND/PID or another device identity into the public package. An unavailable
   original selection must refuse rather than use the foreground window.
2. Admit a source-bound before/after review receipt and its alignment/crop
   representation, using existing package mappings, semantic data and
   `WorkbenchMcpInbox.pixels` where applicable. Bind owner assessments to that
   exact pair and original marker numbers; missing or ambiguous correspondence
   remains unresolved. A returned agent PNG alone carries no capture proof.
3. Decide the compatible origin-reference representation before failure-only
   compilation. Preserve `Package::from_files` validation and existing
   compile/catalog publication APIs; do not relax contiguous numbering or replace
   the compiler. Integration owns any schema/DTO/FFI registration and the new
   `McpOwnerActions`/host-dispatch/tool registration. A subsequent independent
   leaf lane can then test alignment refusal, stale review edits, explicit owner
   verdicts and failed-subset original-number references against those contracts.

## Checks and remaining gates

Ran the existing focused suites with the saved pinned Node `v24.21.0`:

```text
timeout --signal=TERM --kill-after=5s 60s \
  /workspace/.vw-tools/node-v24.21.0-linux-x64/bin/node --test \
  test/bounds.test.mjs test/packages.test.mjs test/protocol.test.mjs
24 passed; 0 failed, cancelled or skipped; exit 0.
```

These are offline/synthetic boundary regressions, including authenticated
loopback HTTP and both MCP protocol eras. No new tests or product code were
needed for this dependency-only result. No heavy build slot was granted or used;
Rust/Kotlin compilation, Windows recapture, phone capture, alignment quality,
owner per-marker review/follow-up and A30 end-to-end were not run. Requirements,
pass/fail targets and gates are unchanged. No verification images were created;
there are none to dispose. Existing drafts and recovery evidence are preserved.

Next: integration resolves the three hooks, then resume T5.01 (L). The plan lists
gpt-6-astra/max; this delegated session uses the inherited settings without an
override, and its exact model/effort is not exposed by a session inspection tool.
Krita remains deferred.
