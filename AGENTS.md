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
