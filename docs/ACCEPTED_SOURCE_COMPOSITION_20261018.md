# Accepted source composition checkpoint

Branch cloud/roadmap-source-integration-20261018 starts from held70475b8, preserving
its complete prior native/bindings/app/JUnit/lint receipts without rerunning them.
The following independently accepted sources are selected unchanged:

- Offlinec2f8f13, five exact granted leaves. Root now calls
  queue_recovered_batch(store.pending()?) once during from_store_owned startup and
  registers session::offline_recovery_tests under cfg(test). No loop of growing
  prefixes remains. The native library compiles with these hooks;15 new runtime
  cases and scale/crash/startup-memory/timing checks remain unrun.
- Desktop350f492b + evidence7f180b48: normal coroutine-recovery fixture correction,
  no production change. Its7/7 focused results are preserved, not a fresh entire
  Desktop pass. All returned Desktop-owned source/test files are byte-identical.
- Androidcf952b05: public Window callback replaces restricted Activity override.
  Exact delegation/lifecycle source is accepted. Owner-reported25 focused cases,
  15 adjacent-file comparisons and isolated lint result do not become a fresh
  whole-app lint/device pass; raw execution receipts were unavailable to reviewer.
  All returned Android-owned source/test files are byte-identical.

Cheap composition checks: git diff --check, rustfmt, exact returned-path byte
comparisons and cargo check --locked -p vw-ffi --lib -j2 pass. No binding generation,
broad app/native test run, packaging, provider or hardware work was repeated.
Main remains6d505416 pending coherent source review/final integration disposition.

The central DEFERRED_VALIDATION_LEDGER_20261018.md carries the actual remaining
commands/journeys. AI/provider owner01a15145-4045-72b2-830e-fe6043d55011 and Paint
owner01a15145-22a1-76b1-ba3e-d2bbe0326006 retain their published file boundaries.
The backend grant isolates real model/runtime implementation from existing
assisted proposal/correction leaves and does not pretend a model is admitted.

Giant79d7d7fc is NOT included in this branch. Its remaining source composition is
narrowly accepted except GP1: blocking regular-file open after pathname metadata
can hang on a FIFO replacement. Root is repairing GP1 on the giant branch with
nonblocking descriptor open and a bounded deterministic race regression. Final
ABI/fourcaller checks remain deferred. Formats02e32f1 stays held for five P2
repairs; f8df49e9 is under re-review, so no gated BMP export or RAW shortcut is
activated. Editing707a92c is fetched only: its handoff reports an Astra disposition
without an independent exact-source review receipt available to root, and its
registration/model/capability/raster/export/package/platform hooks remain undone.

No worktree/archive/failure receipt was removed. Product/physical gates stay open;
compilation does not prove5000-row recovery, editor dispatch or target performance.
