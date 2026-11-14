# Visual Workbench implementation guidance

Rewritten 2026-11-13 by the owner's instruction. Earlier dated owner updates
were superseded and are in Git history; do not revive them.

## Goal and scope

Build a personal G5 version of the product: close M4-KP (Paint only), record
G0–G3 and G5, and finish with an installable MSI and APK for personal use.
There is no release signing; the APK keeps its existing development signer.
Krita, T4.02, T4.05, T4.06, broader editors, full G4, and public signing or
distribution are deferred.

The active sequence is the 2026-11 completion roadmap in
`docs/IMPLEMENTATION_PLAN.md` (Stages 0–5). The product sources are
`docs/BUILD_SPECIFICATION.md`, `docs/REQUIREMENTS.json` and that plan. Current
owner instructions take precedence over all of them.

## Only do what makes the product better (owner rule, 2026-11-13)

Every piece of work, file and artifact must help build, test or run the
product. If it doesn't, don't make it. If it already exists, delete it.

- **No evidence, receipts or archives.** Don't write evidence JSON, receipts,
  before/after snapshot folders, "preserved" drafts, backups or history copies.
  Don't keep them "just in case". This overrides every evidence or receipt
  requirement in `docs/IMPLEMENTATION_PLAN.md` and the other docs.
- **Results go in one line.** Record a test or device result as a short line in
  the commit message or the plan's status for that task (what ran, pass/fail,
  device, and the effort level it ran at). Don't create a separate file for it.
- **Dispose after use.** Build output, captures, screenshots, logs, device
  pulls and scratch folders are deleted as soon as they've served their purpose.
  Keep only what the next build or test step actually reads (for example, a
  downloaded toolchain archive or a model file the build needs).
- **No hidden work.** Don't start side work the owner didn't ask for and the
  product doesn't need, such as audits, reconciliations, history reports or
  extra review rounds.
- **Tell the owner what's left.** At the end of every task, list anything you
  left on disk outside source control that's larger than about 50 MB, and why
  the product needs it. If you can't name a reason, delete it first.
- **Don't leave branches or worktrees behind.** Once a task branch is merged,
  delete the branch and its worktree.

## Process

- Claude Code executes each slice locally on this Windows laptop on a
  `task/<task-id>-<slug>` branch. An independent Claude reviewer agent reviews
  the slice's diff before it merges into main. Review the code, not paperwork.
  One review round per slice.
- **Automatic checks (2026-11-14).** Every push runs `.github/workflows/ci.yml`.
  Nothing merges into main unless its `ci-required` status is green on the head
  commit. The same checks run by hand through `tools/ci/*.sh` on Linux and
  through `build.ps1` on the laptop. A green check is not a device result.
- **Cloud sessions** can't run `build.ps1`, Windows-only code or the phone. They
  work on the branch the session assigns, run the `tools/ci` scripts, read the
  Windows and Kotlin results from the automatic checks, and deliver by pull
  request. Anything that needs the laptop, Paint or the S23 stays pending and is
  said so.
- Standing Git authorization (2026-10-01): commit validated task work, merge
  completed work into main, and push main to origin at each numbered phase
  conclusion, without asking for Git permission again. Never force-push or
  rewrite shared history. Never commit secrets, signing keys, private captures,
  real device serials or network identifiers. The repository is public.
- Owner confirmation is still required for product gates, performance changes,
  live provider spend (OpenAI) and other manual decisions. Record a gate only
  after the owner confirms it.
- Owner pre-authorization (2026-11-03): fetching the assisted-selection model
  archives, and admitting tool or runtime licenses (WiX/jpackage, scrcpy and
  tunnel DLLs, model weights) after each license is shown to the owner and
  confirmed.
- After each slice, report the result, open verification limits, and the next
  planned task from the completion roadmap. Never change app or session
  settings yourself. The one exception is the effort level of delegated work,
  which follows the table below without asking.

## Effort levels (owner instruction, 2026-11-14)

Effort switching is automatic. The main session never changes its own level;
it sets the level of each agent it starts from this table and does not stop to
ask at stage boundaries.

| Work | Runs as | Effort |
| --- | --- | --- |
| Coordination and every stage not listed below | Main session | Its current level |
| Stage 5 core engine, Stage 6 connect without typing, Stage 7 pen path | Delegated agent | `xhigh` |
| Independent review of a slice's code diff | Fresh agent | `high` |
| Anything | | Never `max` |

## Builds and devices

- On the laptop, start with `tools/doctor.ps1`. Build, check and run hardware
  smoke tests through `build.ps1`. License checks run before builds; unknown or
  forbidden licenses fail. Keep dependency pins and lockfiles current, and keep
  `tools/ci/pins.env` in step with them.
- Run one heavy build at a time with at most two build workers, and keep at
  least 10.5 GB of disk free. If a build stalls, diagnose it before retrying.
- Android device: select the S23 Ultra (SM-S918U) explicitly, with no fallback
  to any other device. Install, instrument and clean up as owner user 0, and
  leave other apps and profiles alone. Don't use the OnePlus/IN2019 unless the
  owner says so.
- A compiling skeleton doesn't pass a feature or gate. Physical S Pen gestures,
  target-device performance and owner acceptance stay pending until actually
  performed.
- Never change machine-wide execution policy or security settings, or install a
  driver, without the owner's explicit authorization. Leave unrelated processes
  and installations alone.

## Code conventions

- Rust edition 2024, private unpublished crates, rustfmt, and clippy with
  warnings denied. No `unwrap`/`expect`/`panic` in libraries. Isolate unsafe
  code in FFI or platform modules and explain each unsafe operation with a
  `SAFETY` comment.
- Kotlin: explicit API mode in shared code; keep `commonMain`
  platform-neutral.
- Windows DPI and capture math use physical pixels with Per-Monitor-V2
  awareness.
