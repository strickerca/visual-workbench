# Bounded MCP startup timing — 2026-10-20

Base: `6f7b379be2b697aef1be02abbffe527c4012a1ce`. The Windows lane reports
`mcp_unknown_second_start_timeout` under the unchanged 90-second total smoke
budget. First start, idle, repeated start and first stop completed; the shared
budget expired while awaiting second start. The app exited normally and its
owned process/private-text/runtime cleanup completed. The reported bundle is
approximately 119 MB across 1,514 files per cycle. Raw Windows logs and exact
per-phase durations have not been inspected by cloud.

Source inspection confirms each actual cycle prepares and verifies the full
bundle again. Runtime verification resolves ancestors for each trusted file,
hashes every trusted file and walks the runtime again to reject unlisted files.
That identifies work to measure, not a proven restart deadlock or measured
bottleneck. No verification cache, omitted check or longer budget is introduced.

The actual application-owned producer now records a bounded monotonic startup
trace: parents, manifest, extraction/reuse, markers, inventory, runtime
verification, aggregate service open and ready. `service_open` covers the whole
native factory call; it does not claim a particular native substage stalled.
Each new producer resets its counts and phase durations; repeated start on the
same owner does not create another verification cycle. Completion is recorded
by the producer's own finally, never by a cancelled observer.

The explicit smoke flow emits at most nine supplementary `VW_MCP_TIMING` lines,
one per entered lifecycle step. Fields use only closed step/phase enums,
booleans and bounded counts/durations. They contain no paths, identifiers,
exception text, tokens or arbitrary payload. File and byte counts advance only
after the corresponding SHA256 comparison succeeds. `hashed_files` reaching the
inventory size does not prove the final directory walk or service admission
passed. Times are diagnostic counters clamped to one hour, not acceptance or
pen-latency measurements. A diagnostic sink failure cannot replace the actual
operation result.

Main's 90-second timeout, all two-cycle lifecycle assertions, complete bundle
inventory/hash/path verification, retained startup/cleanup ownership and strict
`VW_DESKTOP_*` collector grammar remain unchanged. Existing failure markers
still reject with their original fixed code; timing output grants no authority
and cannot substitute for a success marker. Main, the native service factory and
the collector source are byte-identical to the base. The local lane's unpublished
Desktop repairs and Android work are outside this change.

Focused cached Kotlin compilation includes all five actual production files and
three test files, using existing compiled dependencies. JUnit runs 25 methods:
22 execute successfully and three existing actual-Windows assumptions skip on
Linux. Tests cover phase/cycle accounting, bounded fields, diagnostic sink
failure, unchanged cancellation/retirement behavior and actual synthetic bundle
hash/unknown-file refusal paths. The collector's 41 tests, three compatibility
checks using actual new timing lines and three offline policy checks pass.
Astra independently clears source. No Gradle/native build, binding regeneration,
packaged runtime, provider, device or Windows action runs in cloud.

Windows should rebuild the source-bound image through its existing serialized
package route, then rerun the ordinary packaged MCP smoke with the same total
budget. Retain the fixed timing lines for first and second start plus first stop:
`step_ms`, `start_ms`, each phase duration, `files`/`bytes` and
`hashed_files`/`hashed_bytes`. A timeout still fails. Compare whether time was
spent in extraction/inventory/hash verification or the aggregate service factory
before selecting an optimization or a separately justified test-budget change.
If hash counts are complete while `bundle_verify` continues, the final path/
inventory walk remains part of that phase. Runtime success and the cause/fix of
the observed timeout remain pending until this replay is measured.

The [machine-readable receipt](evidence/MCP_STARTUP_TIMING_20261020.json) binds
exact source and focused logs. This diagnostic slice passes no product gate.
