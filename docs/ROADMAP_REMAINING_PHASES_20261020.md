# Remaining roadmap phases — 2026-10-20

The remaining roadmap work is in Phases3–5. Phases0–2 are the integrated
software baseline; hardware and owner gates remain open. Exact names and task
requirements come from [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md), as
published in implementation commit53c291e58b8a55512827bd5345b854bf8ec3b16f.
The current parent grant defers Krita despite its appearance in the Phase4 title.

| Phase | Current status | Remaining tasks and boundaries |
| --- | --- | --- |
| **3 — Precision, capture, sync** | Most implementation source integrated, including bounded Desktop receiver closure | T3.01 capture/DPI hardware checks; T3.02 actual Windows/GPU composition, packaged launcher and real stream/freeze/Save/resume/performance; T3.03–05 loupe/lens/selection/tilt device acceptance; T3.06 model/checkpoint/license/runtime/memory admission; T3.07 controls and >=20-document journeys; T3.08 giant platform/GPU/memory/performance; T3.09 PDF/SVG/redaction worker/runtime admission and fidelity/security journeys; T3.10 OS formats/color checks; T3.11 offline scale/carrier switching/phone-AI journeys; T3.12 G3 acceptance |
| **4 — First milestone: remote pen editing in Krita and Paint** | Bounded Paint source integrated; current scope remains partial | T4.01 HEVC/phone frame-rate and latency; T4.03 real input guards/grant/disconnect/retirement; T4.04 ghost-ink behavior; active Paint portions of T4.07 compatibility and T4.08 essential controls; personal-use packaging and physical owner acceptance. Full G4 is deferred |
| **5 — Verification, Android tunnel, release** | Source largely integrated, including explicit root release assembly, current launcher admission and a recovery guide; runtime/admission/release work remains | T5.01 real verification/recapture/alignment and A30; T5.02 provider/model journeys; T5.03 configured DLL/native aggregate/replaceability/notices admission and actual tunnel/device/feedback-loop checks; T5.04 Air Actions/Bluetooth/onboarding; T5.05 diagnostics export/privacy journey; T5.06 admitted Windows toolchain/finalized app image and MSI construction, final packages/licenses/notices/reproducibility/install/uninstall with signing authority separate; T5.07 release-wide A01–A30/compatibility/performance/thermal/recovery and G5; T5.08 optional-format runtime acceptance and dated INSTR-005 owner decision |

Deferred Phase4 scope includes Krita under the current grant, T4.02 lossless text
refinement, T4.05 virtual monitor/driver, T4.06 Photoshop/UXP, broader editor/plugin
work in T4.07, broader palettes in T4.08, and T4.09 full Phase4 acceptance. Do not
automatically reactivate those tasks to finish the current Paint/cloud scope.

Parent accepts the bounded Desktop18158c7 successor. Its six production stream
leaves and eight test/witness files are promoted exactly, with actual root
receiver/draft/panel, Save/resume and retirement wiring. The final125-method
Desktop batch and independent Astra root review pass. Actual Windows/GPU,
composition, packaged launch and real input/performance journeys remain unrun;
see `DESKTOP_STREAM_RECEIVER_INTEGRATION_20261020.md`.

The root integration is published on main as53c291e, including schema3 output
persistence and exact preservation of unadmitted notice proposals. Focused
native/store tests,98 JVM methods, fresh Linux/ARM64 builds, binding generation
and Android instrumentation compilation pass. This is software evidence rather
than completion of a phase, runtime/model admission or physical acceptance.

Windows build/packaging remains blocked here by missing MSVC lib.exe; E artifact
transfer remains CONNECT403 before bytes. Source/runtime/license/physical limits
are tracked in [the execution ledger](ROADMAP_CURRENT_EXECUTION_LEDGER_20261019.md)
and [deferred validation ledger](DEFERRED_VALIDATION_LEDGER_20261018.md).

The [MSI source closeout](MSI_RELEASE_CLOSEOUT_20261020.md) implements the
finalized-image producer under the later overnight grant. Exact WiX/license and
final image admission, Windows execution and actual installer acceptance remain
open. Its new focused batch has134 executed Python passes and four Windows-only
skips on Linux. The separately published Windows metadata repair needs actual
Windows replay; local source and S23 work remain with the Windows task.

The earlier [release source reconciliation](RELEASE_ROOT_COMPLETION_20261020.md) records
the concrete cloud packaging corrections and96 passing Python tests. PowerShell
is absent, so root/wrapper execution and actual Windows assembly remain unrun.
No further independent cloud implementation item was identified within that
earlier grant; the remaining work needs the listed artifacts, runtime/toolchain,
device/owner evidence or a later scope grant. Historical task dependencies do
not authorize reopening accepted source or deferred Krita/broader Phase4 work.
