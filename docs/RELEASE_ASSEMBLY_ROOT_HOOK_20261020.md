# Explicit root release-assembly entry

On base main90c0ee22d5e7d94a56c54068f85bf63ffa481b68, build.ps1 lacked the
exact assembly entry already proposed by tools/release/README.md. The new
assemble-release command forwards the five explicit manifest/hash/destination/
Python/hash arguments to the existing bounded Windows wrapper. It requires
PowerShell7, refuses missing/blank values and release-only flags on other
commands, and limits its timeout to1–3600seconds.

No build or discovery is introduced. Existing assembler input/notice/source
checks remain authoritative. No installer/package, signature, deployment,
runtime admission, GUI/device/provider action or actual product assembly ran.
The separate Gradle/MSI build hook and final package/toolchain/guide admission
remain outstanding.

Actual checks:41 existing synthetic Python assembler tests pass; root offline
IDs/third-party/workspace policy passes. Astra source review finds no actionable
implementation issue. PowerShell is absent here, so PowerShell parsing/execution,
Windows wrapper invocation, NTFS/sharing and process cleanup remain unrun.
See the [receipt](evidence/RELEASE_ASSEMBLY_ROOT_HOOK_20261020.json).

Use PowerShell7 (`pwsh`) for future explicit invocation. The five required root
parameters are ReleaseManifest, ReleaseManifestSha256, ReleaseDestination,
ReleasePythonExecutable and ReleasePythonSha256. Do not supply implicit or
unreviewed runtime/MSI paths; this command only assembles exact already built
inputs and reports missing prerequisites. An assembly completion marker is not
product release readiness or owner acceptance.
