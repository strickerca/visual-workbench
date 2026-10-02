# Visual Workbench implementation guidance

The active repository is this directory. Use `docs/BUILD_SPECIFICATION.md`,
`docs/REQUIREMENTS.json` and `docs/IMPLEMENTATION_PLAN.md` as the v2 product sources.
Root specification/requirements copies are preserved v1 references. Read the
matching task prompt as task guidance; current user instructions take precedence.

Follow the implementation plan's dependency order and keep implementation status
separate from verification. Hardware evidence applies only to the named device.
The IN2019 is authorized for startup smoke tests; S23 Ultra pen/capture/latency
verification remains pending. Do not mark a phase gate or product feature passed
from a compiling skeleton.

Owner update (2026-10-02): defer S23 Ultra physical work and continue software
implementation. The OnePlus/IN2019 is reserved by another project; do not query,
install on, launch on, or otherwise use it until the owner releases that reservation.
Host builds and offline tests may continue. This deferral does not pass hardware
acceptance or waive the Phase 0 gate.

Owner update (2026-10-02, overnight continuation): continue the full implementation
plan without pausing between slices. The connected S23 Ultra (SM-S918U) is now
authorized for project builds, installs and tests as needed. Device-name inventory
is explicitly authorized to select it. Pin automated selection to SM-S918U; never
fall back to the reserved OnePlus. Physical owner gestures and manual gate decisions
remain unverified until performed; continue independent implementation while they
are pending. Report each slice's result and next task/model/effort as work continues.

Owner update (2026-10-02, awake): finish the current implementation slice and pause
after reporting results, the next task, and its recommended model/effort so the
owner can adjust settings. This supersedes the overnight no-pause cadence.
Parallel agent implementation/review is explicitly authorized where it improves
quality and efficiency; assign separate file ownership and serialize heavy builds
and device tests. The S23 authorization and OnePlus reservation remain in force.

Owner update (2026-10-02, full continuation): proceed through the project without
pausing between implementation slices. The OnePlus/IN2019 is connected again and
is authorized for applicable project tests; the prior reservation is released for
this work. The S23 Ultra is being taken away, so defer all checks specifically
requiring it. Select IN2019 explicitly, with no fallback to another device. Keep
OnePlus software evidence separate from S23/S Pen, target-device performance and
owner acceptance. Continue software using the documented provisional designs
where hardware-only dependencies are deferred; keep those decisions and product
gates pending. Do not invent pen measurements, preferences or gate approval.
This supersedes the awake pause cadence and the earlier OnePlus reservation.
Parallel implementation remains authorized; preserve other apps and processes.

The owner shares the IN2019 with another app development session. Backgrounding
can be normal. Check actual foreground state, use bounded starter-only relaunches,
and preserve the other app and its processes. Do not reset adb or the device to
recover a focus change. A persistently contested UI run is inconclusive.

Use Rust edition 2024, private unpublished proprietary crates, rustfmt and clippy
with warnings denied. No unwrap/expect/panic in libraries; isolate unsafe code in
FFI/platform modules and explain each unsafe operation with a SAFETY comment.
Use explicit Kotlin API mode in shared code and keep commonMain platform-neutral.
Keep Windows DPI and capture math in physical pixels with Per-Monitor-V2 awareness.

Build through `build.ps1`; license checks run before builds. Keep dependency pins
and lockfiles current. Unknown/forbidden licenses fail. Never add secrets, signing
keys, private captures, real device serials or network identifiers to source/evidence.

Run one heavy build at a time, at most two build workers. Use bounded process-tree
execution with progress and retain failure receipts; diagnose a stall before retry.
Preserve unrelated processes, installations, files and worktrees. Never change
machine-wide execution policy, security settings or install a driver without the
owner's explicit authorization.

Standing owner authorization (2026-10-01): this project's GitHub repository is
public. Commit validated task work locally and integrate completed dependency
work without repeated Git permission questions. At the conclusion of each
numbered phase, automatically commit all phase-owned source/evidence and push
main to origin, following docs/IMPLEMENTATION_PLAN.md "Automatic phase publishing".
This covers Git publication; owner confirmation of product gates, performance
changes and other manual decisions still follows the plan.
Preserve unrelated work, stage explicit phase-owned paths, scan staged bytes,
exclude local caches/secrets/signing material/verification images, and never
force-push or rewrite shared history. Report the commit, remote URL and verified
remote SHA, or the concrete blocker if publication cannot finish.

Verification screenshots go in an owned temporary directory outside the repository.
Inspect and then dispose them before closure; retain only text results, counts,
hashes, source bindings and a disposal receipt. Product image assets are preserved.

After each implementation slice, report its result and open verification limits,
then name the next planned task, task size, recommended model and reasoning effort
from that task prompt. Settings are changed by the owner; verify the actual
session settings and report mismatches instead of changing them automatically.
