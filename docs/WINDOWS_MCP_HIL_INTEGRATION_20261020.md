# Windows MCP/HIL integration — 2026-10-20

This merge composes the reviewed diagnostics/shared repair published as
`a7674f4eddccafbec03e7776fa070f68f8ef6146` with the exact Windows branch
`893400746ca87774ae182093e053660e94db33b2`, tree
`e89227bdcce0b2c1f3314167c1963b29cb0c8c7f`. The latter descends from main
`3bb8b7a6329d3c040dffbd885af3ec5b1b61a584` and includes MCP
`e7b10cb83aef213d8ba8abb9a20589be5c87f962`. Both histories are retained.
The separate unpublished Desktop repair is outside this merge.

All 17 accepted Windows leaves are byte-identical to that published head.
The four diagnostics/shared code and test leaves remain exact to the reviewed
repair. Its documentation now correctly calls the shared runner unchanged;
the bound document hash is refreshed. No accepted source is overlaid from an
unpublished local candidate.

MCP smoke failures now retain a fixed step and reason without exception text,
paths or identifiers. The existing deadline, two lifecycle cycles, identity,
duplicate-owner, idle/no-grant/no-send and retirement assertions remain intact.
All recognized failure markers still reject. This improves failure diagnosis;
it does not establish a packaged MCP lifecycle success.

Android HIL preflight inventories all 225 distinct current source methods and
retains the historical baseline, additions and retired-method accounting. It
requires the exact runtime starts and successes. Cleanup preserves an original
instrumentation failure and retains uncertain installer/private-work ownership.
The two narrow Android production changes explicitly disable tile filtering and
avoid a duplicate gesture cancellation; instrumented fixtures retain actual
cleanup and cancellation assertions.

The accepted commit records a real SM-S918U replay of 225 started, 222 passed,
three failed, none skipped, with task-owned cleanup complete. These are relayed
Windows-lane results, not raw device logs inspected or device actions executed
by cloud. The remaining surface-wait, sync and canvas-cancellation failures stay
with that lane; this merge does not fix or waive them. The older failing replay
and its cleanup history remain preserved in the local evidence.

Fresh cloud checks pass 31 Android HIL parser tests and 41 desktop collector
tests. The preceding diagnostics/shared focused run passes 22 JVM methods.
Three offline policy checks pass. Astra independently clears source integration,
exact accepted-leaf preservation, the strict failure/ownership predicates and
225-method source inventory. Cloud does not execute PowerShell cleanup fixtures,
Win32 publication, packaged runtime, Android instrumentation or a heavy build.
No binding/ownership regeneration or provider call runs.

Windows should replay `DiagnosticsPublicationTest`,
`WindowsDiagnosticOwnedFileTest` and shared `RemoteCompatibilityImportTest` from
the final integration head. Continue existing local MCP/runtime, managed-build,
Android and installer-admission work serially. Retain new failure traces and
keep all physical, runtime and product acceptance gates separate.

The [machine-readable receipt](evidence/WINDOWS_MCP_HIL_INTEGRATION_20261020.json)
binds the exact accepted leaves, preserved repair and fresh test logs.
