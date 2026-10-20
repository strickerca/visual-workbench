# Finalized-image MSI production

`build.ps1 package-desktop-msi` builds a per-user MSI from the finalized
`VisualWorkbenchDev` app image. It consumes explicit reviewed inputs; it does
not build the image, find/download tools, install a package, accept a license
agreement or sign anything.

## Windows prerequisites

Use the final producer source, then finalize the image through the existing
`package-desktop-app` route: `createDistributable`, `Complete-McpApplicationImage`,
then the desktop receipt. The complete image and normal Android APK must pass
the existing desktop/personal-package source, resource, native and notice
checks. The desktop receipt binds `build.ps1` and `tools/process.psm1`; it cannot
be relabeled after those files change.

Supply a complete ordinary-file inventory of the Windows JDK21, WiX3 and base
Python installations, plus separate inventory, compatibility and license-review
evidence. DLLs, modules, WiX extensions/configuration and Python runtime/stdlib
belong in the inventory. Review any WiX components incorporated into the MSI as
well as the tools. A file named `license-review.txt` does not approve its contents.
The explicitly supplied manifest SHA256 is the root admission handoff.

The [JDK21 packaging documentation](https://docs.oracle.com/en/java/javase/21/jpackage/packaging-overview.html)
requires Windows packaging on Windows with WiX. Its
[pipeline uses Candle/Light](https://github.com/openjdk/jdk21u/blob/master/src/jdk.jpackage/windows/classes/jdk/jpackage/internal/WixPipeline.java);
a later-family `wix.exe` is not a drop-in replacement for this profile.
Windows reports installed jpackage `21.0.12.1`, SHA256
`afd944cbf952a461d283a7df7714c12cdc05fd29441eae04526966d71357e9a2`,
and JDK `release` SHA256
`07117c72ce033949c14878e07fbf2fe23f59a1f8c90a6d1351b5c89099847ce7`.
These parent-relayed observations are not a complete admitted toolchain.

Windows reports no WiX installation/license inventory. The official
[WiX3.14.1 release](https://github.com/wixtoolset/wix3/releases/tag/wix3141rtm)
offers `wix314-binaries.zip` separately from the installer; its
[MS-RL terms](https://docs.firegiant.com/wix3/main/license/) still govern use.
WiX3 is [out of community service](https://docs.firegiant.com/wix/wix3/).
No tool admission or new license acceptance is established by this source.
The cloud GET of `https://api.github.com/repos/wixtoolset/wix3/releases/tags/wix3141rtm`
failed at proxy HTTPS CONNECT with403 before HTTP response bytes. No archive
download or alternate transfer route followed. The existing complete app-image
package remains the supported delivery path requiring no WiX; it does not
establish MSI or full release acceptance.

## Exact input contract

The toolchain JSON has exactly `schema`, `jdk`, `wix`, `python`, `evidence`.
`schema` is `vw-msi-toolchain-v1`. Each tool object has exactly `root`, `version`,
`roles`, `files`. Roots are explicit ordinary absolute paths. `files` is the
complete array of `{path, bytes, sha256}` descriptors relative to that tool root.
Unknown files, case aliases, reparse points, hard links, overlaps, PATH separator
escapes and changes are refused. Do not substitute illustrative or guessed hashes.

| Object | Exact roles and supported layout |
| --- | --- |
| `jdk` | Version21.x; `roles` is `{"jpackage":"bin/jpackage.exe","java":"bin/java.exe"}` |
| `wix` | Exact reviewed3.x version; roles `candle` and `light` point to `candle.exe` and `light.exe` in the same directory |
| `python` | Exact installed3.10+ version; roles `{"python":"python.exe"}`; matching `python3<minor>.dll` and `Lib/os.py` or stdlib ZIP; no `pyvenv.cfg`, `._pth`, nested executable or parent virtual environment |
| `evidence` | Exactly `inventory`, `compatibility`, `license_review`; each is a nonempty checkout-relative `{path, bytes, sha256}` file descriptor |

Root flags are `-MsiToolchainManifest` (checkout-relative),
`-MsiToolchainSha256` (reviewed hash), `-MsiWorkDirectory` (fresh absolute path
outside the checkout/tool roots) and optional `-TimeoutSeconds` (1–3600 for the
jpackage process). The work parent must exist. Optional `-RemoteEditorCatalog`
and `-RemoteEditorCatalogSha256` are an exact checkout-relative path/hash pair.
Use PowerShell7.2+. MSI-specific flags on other root commands are refused.

## Ownership, output and validation

`plan` validates inputs and exclusively creates `plan.json`, `dest`, `temp` and
`runtime-temp` in fresh work. The wrapper pins all known inputs/ancestors before
`check`, retains them over one bounded jpackage process and writes its exact
invocation/plan-bound proof to `process.json`. The explicit child environment
contains only planned JDK/WiX PATH/JAVA_HOME/temp values and Windows OS paths
obtained from the special-folder API. Existing callers of the process runner
retain their previous environment behavior.

Metadata is fixed: `VisualWorkbenchDev`, `0.0.1`, vendor `Visual Workbench`,
per-user installation and a Start Menu entry. Upgrade UUID
`127902df-9261-5ead-8845-df9bc2cd90b4` is UUIDv5 of
`https://github.com/strickerca/visual-workbench/installer/windows/VisualWorkbenchDev`.
Keep it stable for this development package family; no signing/device identity
is changed.

`record` rechecks inputs, tools and namespaces; requires exit0/parent0, job
containment, EOF, confirmed cleanup and no survivors; and admits exactly
`dest/VisualWorkbenchDev-0.0.1.msi`. It binds bytes/hash and bounded OLE-CFB header/
sector shape to the plan/process, then exclusively creates `receipt.json`.
Temp inventories and failed work are retained; no recursive cleanup runs.

Known-file leases and snapshots cannot exclude every transient namespace
writer. Atomic namespace, MSI database, payload, signature, install/launch and
product acceptance stay explicitly unverified. Windows must inspect the actual
database/payload, compare it with the admitted image and perform applicable
installation, uninstall, recovery and launch checks.

First run `tools/release/test_msi_wrapper.ps1` through PowerShell7.2+ on Windows.
It parses the real root/wrapper/module, tests actual bootstrap functions and
runs two short owned PowerShell children for explicit environment and legacy
caller behavior. It runs no MSI, app or device. Python tests use synthetic files
and scoped app/host identity seams; they do not establish installer acceptance.
