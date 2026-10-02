# T0.08 driver preparation and recovery plan

This is an incomplete, byte-pinned source preparation, not an installable driver.
See LICENSES.md and REVIEW.md. `check_source.py` accepts the reviewed source
inventory; `check_source.py --build` rejects the unresolved EDID provenance gap.
The three cached NuGet packages are development tools only and remain ignored.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File drivers/sudovda/preflight.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File drivers/sudovda/preflight.ps1 -RequireBuildReady
```

Preflight hashes all cached packages, checks the selected MSVC Spectre library,
and attempts read-only Secure Boot, running HVCI and restore-point queries. Errors
remain unknown, never false or passed. It creates no restore point and changes no
machine configuration. The second command fails until build prerequisites close.

Remaining implementation and owner sequence:

1. Resolve the EDID data's license/provenance or replace it with permitted data.
   Update the source manifest with an explicit reviewed fork change; no local
   cache file is implicitly approved for vendoring.
2. In Visual Studio Installer, modify Build Tools and add **C++ Spectre-mitigated
   libraries for x64/x86 (Latest MSVC)**. MSVC 14.50.35717 is selected here.
   Re-run preflight. Never set Spectre mitigation to false to make the build pass.
3. Complete the MSBuild wrapper using the three matching 10.0.28000.2526 WDK/SDK
   packages. Microsoft demonstrates their imports in Directory.Build.props. Keep
   output/intermediate files ignored, use Release/x64 and at most two workers,
   and explicitly disable automatic build-time signing. A full driver build is
   still unverified; no install/uninstall script is represented as completed.
4. In an owner-approved administrator session, verify Secure Boot true, HVCI
   running, and System Protection enabled. Create/confirm a recent restore point
   before any certificate trust or device installation. Capture Code Integrity
   events before/after, retaining event IDs/times and redacted driver-specific
   error text. Do not log unrelated event details.
5. Create the project-only exportable code-signing certificate only when actual
   signing is ready. Retain only its public certificate in the separately built
   package; no private key enters files in this repository. Stamp the INF, run
   Inf2Cat and Authenticode-sign the driver/catalog with an RFC3161 timestamp.
   Verify exact file hashes, codeSigning EKU and signatures before installation.
6. The future installer must journal each owned change before performing it:
   the exact certificate thumbprint with its prior presence in each trust store,
   the newly published oemNN.inf package, and the exact generated device instance.
   Do not infer ownership merely from a display name. PnPUtil `/add-driver`
   stages a package; a root/software device must also be created for this INF.
   DevGen from the owner's WDK can create a development test device, but cannot
   ship with the application. A pre-existing SudoVDA installation or certificate
   must cause refusal rather than being taken over.
7. Exercise both probe modes only after owner approval of driver/trust changes.
   Reserve the entire shared watchdog instance. Record actual add/remove/kill
   timing, Settings visibility, window return and security state before/after.
8. If policy rejects the driver, retain the exact error. Test the separately
   signed VirtualDrivers fallback only after verifying a stable release's assets,
   signatures, license and named-pipe protocol. It is not downloaded, installed,
   or called compatible merely because its README says signed.
9. After all signing, the owner exports the exact certificate's private key to
   password-protected offline storage in their own terminal. The password never
   enters this chat or a transcript. Confirm the export exists, then delete that
   exact certificate/private key from CurrentUser\My, and verify absence. No
   signing key was created during this checkpoint.

Rollback must be implemented and tested with the installer journal before the
first real installation. Its ordered operations are:

- Stop only project-owned display clients. Remove only their recorded monitor
  GUIDs; preserve unrelated clients and displays.
- Remove the exact journaled DevGen device instance, then the exact newly
  published `oemNN.inf` package using PnPUtil `/delete-driver ... /uninstall`.
  Do not use wildcards, `/force`, broad display-class removal, or automatic reboot.
- Verify the instance and package are absent. Remove the exact project public
  certificate from LocalMachine\Root and LocalMachine\TrustedPublisher only
  where the journal proves that this install added it. Preserve pre-existing
  trust entries. Retain a partial-failure receipt instead of claiming rollback.
- Recheck Secure Boot/HVCI and inspect bounded Code Integrity events. If Windows
  requests reboot, ask the owner; do not reboot this shared machine unattended.
- If the display becomes unusable, use the recorded restore point or Safe Mode
  to remove the exact journaled device/package. Keep the recovery record outside
  the installed package so removal cannot erase it.

No test-signing, Secure Boot, HVCI, certificate trust, registry, restore point,
driver-store or display-topology change has been performed by this checkpoint.

Sources checked 2026-10-02:
- [Official WDK NuGet setup](https://learn.microsoft.com/en-us/windows-hardware/drivers/install-the-wdk-using-nuget)
- [Microsoft package-import example](https://github.com/microsoft/Windows-driver-samples/blob/main/Directory.Build.props)
- [DevGen command syntax and redistribution restriction](https://learn.microsoft.com/en-us/windows-hardware/drivers/devtest/devgen-command-syntax)
- [VirtualDrivers fallback](https://github.com/VirtualDrivers/Virtual-Display-Driver)
