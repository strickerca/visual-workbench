# Visual Workbench

Read and follow AGENTS.md; it holds the current rules. Product sources are
docs/BUILD_SPECIFICATION.md, docs/REQUIREMENTS.json and
docs/IMPLEMENTATION_PLAN.md, whose active roadmap sets the order of work.

- Every push is checked by .github/workflows/ci.yml. Nothing merges into main
  unless `ci-required` is green, and an independent reviewer agent reviews each
  slice's code diff once.
- On the laptop: start with tools/doctor.ps1 and use build.ps1 for builds,
  checks and hardware smoke tests, on task/<task-id>-<slug> branches.
- In a cloud session: run tools/ci/*.sh, work on the session's branch and
  deliver by pull request.
- No evidence files, receipts or archives. Results go in one line in the commit
  message or the plan's task status, including the effort level used.
- The standing Git authorization (2026-10-01) and the effort-by-stage table are
  in AGENTS.md. Product gates, performance changes and live provider spend
  still need the owner.
