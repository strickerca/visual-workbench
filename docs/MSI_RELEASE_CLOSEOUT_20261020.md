# MSI source closeout — 2026-10-20

The new overnight grant makes the remaining MSI implementation independent
cloud work, with actual Windows/toolchain and S23 checks owned by the local
Windows task. This supersedes the earlier bounded release record's statement
that no further independent source item was identified under its previous grant.
It does not activate deferred editor expansions or pass a phase gate.

`build.ps1 package-desktop-msi` now drives an explicit finalized-image producer.
The Python backend validates supplied tool inventories and current image/APK/
source/notice receipts, creates a fresh plan, rechecks it and records actual MSI
bytes/hash and bounded OLE-CFB shape after the process. The PowerShell wrapper
retains known-file leases over validation/execution/recheck, uses the explicit
process-runner environment and binds the exact invocation to exit/containment/
EOF/cleanup proof. It never substitutes an installer task before the final MCP
bridge is copied. The accepted Desktop renderer and application code are unchanged.

The bootstrap validates complete base Python layout before executing it,
refusing alternate import paths, virtual environments and PATH separators.
JVM runtime temp is separate from jpackage's required empty staging directory.
Failed work is retained. Input/tool paths and actual inventory evidence belong
in private local manifests; public evidence contains source/log hashes and counts.

Fresh Linux validation runs35 MSI methods:33 pass, two real Windows host-identity
witnesses are skipped. The fixtures scope only synthetic app admission and host
identity seams; they retain production file, process, namespace and environment
checks. The separate repaired assembler runs48 methods:46 pass, two actual Windows
metadata witnesses are skipped. The39 desktop-report and16 personal-package
methods pass. These are134 executed passes and four Windows-only skips across
138 methods, not actual Windows tool or installer execution. PowerShell parsing
and the16-case wrapper fixture still need the Windows task; its two short owned
PowerShell children run no app, MSI or device.

The [input contract](../tools/release/MSI_INPUTS.md) records the parent-relayed
installed JDK observations, exact role/manifest/root flags and process/output
boundaries. Windows has no admitted WiX installation/license inventory or final
app-image receipt yet. The specific cloud WiX metadata request was proxy-denied
at CONNECT403; no alternate artifact route followed. The existing complete
app-image package remains available without WiX. New legal agreement acceptance,
signing and deployment require their actual authority.

The receipt leaves atomic namespace, MSI database/payload, signature,
installation/launch and product acceptance false. Actual Windows must verify
those applicable properties, including installed payload against the admitted
image and project-preserving uninstall/recovery. The new root/process source
requires a fresh matching app-image receipt; older evidence cannot be relabeled.
See [the source-bound receipt](evidence/MSI_RELEASE_CLOSEOUT_20261020.json) and
the separate [Windows metadata repair](WINDOWS_RELEASE_METADATA_20261020.md).
