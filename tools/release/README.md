# Explicit release assembly

This source batch assembles already built, explicitly listed bytes into a fresh
directory. It does not search for runtimes, build an MSI/APK, download, sign,
install, launch a package, or interact with a device. Unavailable licensed
decoders, runtime/composite inputs and package inputs remain external
dependencies; their absence is not a successful package check.

`assemble.py` uses Python 3.10+ and the standard library. It loads the existing
repository `tools/desktop-test/reports.py`, `personal_package.py` and
`tools/ffi-test/check_apk.py`; it does not load code named by the input manifest.
The Windows wrapper requires PowerShell 7, the existing bounded process runner,
and an explicit interpreter path and SHA256. The wrapper is orchestration source
for later root review; it has not been executed on Windows in this batch.

## Input contract

All input names are exact paths relative to `--root`. Paths use `/`, with no
globs, traversal, Windows device names, alternate streams, case aliases,
symlinks, reparse points, or hardlinked files. The root and destination must be
ordinary absolute paths with existing ordinary ancestors. The destination's
parent must exist; the destination itself must not exist, including as an empty
directory or dangling link. A destination inside a source directory is allowed
only if it overlaps none of the explicitly read files.

The manifest must use the required keys below. `release_documents` and
`external_dependencies` are optional bounded keys. Hash and size placeholders are
descriptors to fill from actual admitted inputs, not executable examples:

```json
{
  "schema": "vw-release-assembly-v1",
  "release_id": "reviewed-20261018",
  "binding": {"path": ".local/release-binding.json", "bytes": 123, "sha256": "<64 lowercase hex>"},
  "artifacts": [
    {"path": "target/reviewed/output.bin", "destination": "artifacts/output.bin", "bytes": 123,
     "sha256": "<64 lowercase hex>", "kind": "opaque"}
  ],
  "notice_manifest": {"path": "third_party/LICENSES", "bytes": 123, "sha256": "<64 lowercase hex>"},
  "notices": [
    {"path": "third_party/notices/ACTUAL-NOTICE.txt", "bytes": 123, "sha256": "<64 lowercase hex>"}
  ],
  "catalog": null,
  "release_documents": {
    "toolchain_record": {"path": "docs/TOOLCHAIN.md", "bytes": 123, "sha256": "<64 lowercase hex>"},
    "uninstall_recovery_guide": null
  },
  "external_dependencies": [
    {"id": "licensed-decoder", "reason": "No reviewed licensed binary was supplied"}
  ]
}
```

The hash-pinned binding JSON must use exactly these keys. `artifacts` must match
every manifest artifact's source path, size and hash, with no extra or missing
artifact. `sources` lists the actual source/evidence bytes bound to those
artifacts. A source revision is a 40-character lowercase commit label.

```json
{
  "schema": "vw-release-source-binding-v1",
  "source_revision": "<40 lowercase hex>",
  "sources": [
    {"path": "actual/source/file", "bytes": 123, "sha256": "<64 lowercase hex>"}
  ],
  "artifacts": [
    {"path": "target/reviewed/output.bin", "bytes": 123, "sha256": "<64 lowercase hex>"}
  ]
}
```

The assembler checks the exact declaration and current source/artifact hashes.
It does not authenticate the commit label or prove that arbitrary build outputs
were compiled from declared sources. Root must admit the build evidence and
complete source closure before treating this association as build provenance.
There is no alternate generic build-receipt authority in this tool.

T5.06's toolchain record and uninstall/recovery guide are explicit delivery
slots, rather than inferred from an arbitrary source list. Each slot is `null`
or an exact nonempty descriptor bounded to 8 MiB. Supplied bytes are copied
unchanged to `release-docs/TOOLCHAIN.txt` and
`release-docs/UNINSTALL-RECOVERY.txt`, included in checksums and recorded as
`supplied-hash-verified-content-not-accepted`. Missing slots, including an
omitted `release_documents` object, are recorded as
`missing-release-prerequisite`. Root must review the actual record's pins,
producer evidence and guide content (including driver/certificate removal and
project backup); the assembler does not invent actual producer tool versions,
perform removal, or claim guide acceptance. `release_ready` is always `false`.
Optional `external_dependencies` records supplied missing licensed decoder,
composite, runtime or package prerequisites without treating them as completed:
at most 64 unique lowercase slug IDs and 512 ASCII characters per reason. A
missing listed artifact itself still refuses assembly.

Artifact kinds have these exact meanings:

| Kind | Admission |
| --- | --- |
| `opaque` | Exact nonempty already built bytes and source association only. |
| `external-package` | `.msi` bytes and source association only; MSI structure, installer behavior and signatures remain unverified. |
| `apk` | `.apk` bytes plus the existing bounded, extraction-free `check_apk.verify_apk` native/ELF/ZIP checks on the copied snapshot. No Android source-build/signature/installation claim. |
| `desktop-file` | A file under the existing fixed `reports.DIST` app-image path. Requires the complete existing admitted app image with its relative layout preserved under one `artifacts/.../` directory. |

APK and MSI source suffixes require their corresponding kind. Every source path
under the fixed desktop app image requires `desktop-file`. An opaque file's
contents are not classified or given package acceptance.

Desktop admission reuses `reports.check` and `personal_package.verify`, including
the existing exact source/build nonce, native/runtime/MCP resources and notice
checks. It also requires the normal debug APK as an explicit `apk` manifest
artifact because the existing personal-package verifier reads it. The source
root must be the checkout containing these validators for desktop admission,
because the existing personal-package checker imports its APK checker there.
The source
binding must explicitly include **all** checked `built['source']` entries,
`target/debug/` native inputs, and the exact `reports.BEGIN` / `reports.BUILT`
receipts. Unlisted source/build/native inputs refuse. An admitted catalog is
either `null` or a normal descriptor for its exact path/bytes/hash, bounded to
32 KiB; it has no action-authority claim.

The `notices` list must include precisely the files declared by the copied,
hash-pinned `third_party/LICENSES`. The existing `personal_package.notices` checks
metadata, exact hashes and complete copied notice inventory. Full notice bytes
are delivered along with the source manifest. No license-compatibility decision
is made by this assembly step. Preserve all actual runtime/legal files when a
desktop image is delivered.

Bounds: 8 MiB per JSON or generated metadata file; 4,096 artifacts; 2 GiB per
artifact; 8 GiB total artifacts; 24,000 source/evidence entries; 2 GiB per source
file and 4 GiB aggregate source/evidence bytes; 512 nonempty notices, 4 MiB each
and 32 MiB total; 32,768 pinned directory identities; 32,768 total output
namespace entries including directories and the staging root. Names are bounded to 512
ASCII characters and 24 components by the existing path contract. Operating
system handle/resource exhaustion is a refusal, not a partial success.

## Output and failure behavior

The freshly created destination is the task's staging directory. A successful
assembler result and `COMPLETE` containing the exact `inventory.json` hash are
both required for an assembly receipt. A failed run's retained tree is
incomplete. Directory creation and publication of the whole tree are not an
atomic directory transaction. Each file is created exclusively; inputs are
streamed in 1 MiB chunks. The assembler checks opened-file identity, size,
mtime/ctime and link count around reads, then rereads original declared inputs
and copied files after package verification. POSIX operations use pinned
ancestor descriptors and no-follow relative opens; Windows reads use no-follow,
read-only file leases and pinned no-delete directory handles. Windows behavior
still needs real host validation. Bounded, no-follow scans check the entire
output namespace against the recorded files and owned directory identities,
including empty directories. Unknown files, directories and links refuse.
Expected files must retain their recorded identity/size/time/link snapshots;
expected directories must retain their recorded identities. Each scan checks
directory metadata before and after enumeration and rechecks its ancestor
chain. These scans run before `COMPLETE` is created and again afterwards,
including the marker in the final gate.

The successful final observed namespace must match:

- Listed artifacts under `artifacts/`, preserving a complete desktop layout.
- Actual `third_party/LICENSES` and `third_party/notices/*` input bytes.
- Exact `release-manifest.json` and `source-binding.json` input snapshots.
- Explicit supplied toolchain record and uninstall/recovery guide under
  `release-docs/`, with missing slots recorded in the inventory.
- `THIRD-PARTY-NOTICES.txt`, with actual delivered notice hashes and source URLs.
- `inventory.json`, with actual input/output hashes, reused package-check
  receipts, and explicit limits on the claims.
- `SHA256SUMS`, covering every preceding delivered file including the inventory.
- `COMPLETE`, containing the inventory hash. Neither checksum file claims to
  recursively hash itself; `COMPLETE` and `SHA256SUMS` are excluded from the
  inventory's `files` map to avoid a circular digest.

On refusal after destination creation, the entire staging tree is retained on
all platforms. Cleanup performs **no pathname deletion**. POSIX cannot unlink
by a retained inode; checking an inode and then unlinking its name can delete a
foreign replacement. The current backends do not prove exclusion for every
output entry, so even apparently unchanged owned files/directories remain.
The original refusal is preserved; a retained destination cannot be reused for
a later run.

The assembler retains a descriptor to its own `COMPLETE` file. On refusal after
marker creation it truncates and fsyncs **that object**, leaving an empty,
invalid marker when it still occupies the output path. It never deletes or
truncates a replacement found at the marker's pathname. If another writer moves
the owned marker and supplies a foreign replacement, the owned descriptor is
invalidated and the foreign path is preserved; a marker alone does not override
the failed process result. Invalidation failure is explicitly reported as
`incomplete_marker_invalidation_failed`.

These gates establish an observed namespace snapshot, not atomic exclusion of
concurrent writers. A mutation after a checked entry or after the last scan can
still occur; the marker can be observed before the final gate finishes. Delivery
consumers must use the successful process result and revalidate the complete
namespace and hashes after writer access has stopped. The tool does not provide
an atomic directory publication or a tamper-proof completion claim. Existing
destinations and source files are preserved. A successful assembly receipt is
not signature, installation, startup, editor, device, performance or physical
acceptance. Unexpected I/O/validator errors cross the CLI as a static refusal
code without host paths.

## Focused checks

```bash
PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s tools/release -p test_assemble.py -v
```

Fixtures contain synthetic source/notices/build bytes and reuse the existing
synthetic APK fixture builder. They exercise copied inventories/checksums,
bounded admission, actual notice gates and native APK validator reuse, malformed
inputs, source/output identity replacement, source mutation, no-follow paths,
hardlinks/FIFOs, simulated reparse attributes, fresh-output preservation and
conservative failure retention. Regressions exercise the stale-stat / foreign
replacement cleanup race without any unlink, unlisted DLL/empty-directory/link
injection during validation, bounded namespace enumeration, final-gate file
replacement and retained-descriptor marker invalidation. These checks do not
build or package the product.

## Explicit root assembly entry (2026-10-20)

Root owns `build.ps1`, Gradle/MSI configuration and pinned runtime staging. The
`assemble-release` entry in `build.ps1` forwards these five explicit root
parameters and the existing timeout to the bounded wrapper:

```powershell
[string]$ReleaseManifest,
[ValidatePattern('^[0-9a-f]{64}$')][string]$ReleaseManifestSha256,
[string]$ReleaseDestination,
[string]$ReleasePythonExecutable,
[ValidatePattern('^[0-9a-f]{64}$')][string]$ReleasePythonSha256
```

Run this command with PowerShell7 (`pwsh`), not Windows PowerShell5.1. All five
inputs must be explicitly bound and nonblank; both hashes require64 lowercase
hex digits. Release-only parameters are rejected on every other root command,
including explicitly bound empty values. The assembly timeout is1–3600seconds.
The root command delegates to the bounded wrapper:

```powershell
'assemble-release' {
    & (Join-Path $projectRoot 'tools/release/package-windows.ps1') `
        -Root $projectRoot -Manifest $ReleaseManifest `
        -ManifestSha256 $ReleaseManifestSha256 -Destination $ReleaseDestination `
        -PythonExecutable $ReleasePythonExecutable -PythonSha256 $ReleasePythonSha256 `
        -TimeoutSeconds $TimeoutSeconds
}
```

The entry adds no implicit build or runtime/MSI discovery. It requires an
existing hash-bound manifest/source record, and the assembler independently
checks exact inputs, notices and available package validators. The caller must
still supply genuinely admitted artifacts; hashes do not self-admit a runtime. The separate
root admission must populate the toolchain-record/guide slots with exact reviewed
bytes or retain the explicit missing-prerequisite statuses. The
root MSI build hook remains deferred until the parent reviews its exact Gradle
task, configuration, dependencies, license gates, runtime pins and output
receipt. Once that hook produces a real already built MSI, list its exact bytes
as `external-package`; this does not pass MSI validation. The new root entry
is source-reviewed by Astra. Its underlying synthetic Python suite passes41
cases and root offline policy passes. PowerShell is absent in the cloud, so
PowerShell parsing/execution and Windows wrapper/NTFS/process-tree checks remain
unrun. No product assembly, package, installer, signature or deployment was
produced by this change. See `docs/RELEASE_ASSEMBLY_ROOT_HOOK_20261020.md`.

Still deferred: real Windows/NTFS reparse, junction and sharing-violation tests;
PowerShell parsing/execution and process-tree failure cleanup; actual complete
desktop/native/MCP/runtime resource closure; full license/dependency policy;
licensed decoder/composite/runtime availability; root/MSI build admission;
MSI integrity and installation/uninstallation; signing and authenticated
signature verification; installed startup/editor behavior; device, S Pen,
performance and owner acceptance. Missing inputs cannot be substituted with a
fixture or an invented receipt.
