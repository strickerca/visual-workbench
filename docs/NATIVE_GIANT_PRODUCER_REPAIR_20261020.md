# Native producer regression handoff

This candidate is based on Windows managed-green `c241229371517e96a9d98033e43a0c68adb72ea1`.
It changes two test files. No producer ownership, path-admission, quota, cancellation,
publication, cleanup or production DTO implementation changes.

The Android-copy fixture now creates its owned temporary directory under the
canonical temporary parent. `checked_output_root` deliberately requires canonical
path equality. Rust documents that Windows `canonicalize` uses extended-length
path syntax, while the previous fixture retained the original temporary path.
The fixture correction preserves that strict production check. This explains a
plausible source of the three Windows `Invalid` failures; actual Windows replay
must establish causality. See the [Rust canonicalize documentation](https://doc.rust-lang.org/std/fs/fn.canonicalize.html).

The three verification-coordinate comparisons use boolean equality assertions
with fixed diagnostic messages. They still compare the complete package pair and
both package keys. No `Debug` implementation was added to production DTOs.

## Focused cloud evidence

The pinned Rust toolchain built the lib-unit and verification integration targets
offline with one worker, a 240-second bound and a 12-GiB virtual-memory ceiling.
An initial successful compile included an optional new alias assertion; that
assertion was removed before the final successful compile and all tests below.
Both compile logs are retained outside the repository. The final exact binaries
are identified by Cargo's successful JSON artifact records and SHA256 hashes.

- Producer filter: 62 passed, zero failed, one top-level ignored entry, 418
  filtered out; one test thread. All seven reported producer regression names
  executed and passed on Linux.
- The ignored entry is exactly the quarantine child probe. Its bounded parent
  wrapper ran and passed; the wrapper verifies the exact child name, successful
  exit and one executed passing child case. The retained real admission and
  original 4-MiB charge are tested inside that child. No counter reset or fake
  credit was introduced. The raw top-level ignored count remains one.
- Verification integration: two passed, zero failed, zero ignored or filtered;
  one test thread.

See `evidence/NATIVE_GIANT_PRODUCER_REPAIR_20261020.json` for source/binary/log
hashes and the seven exact regression names. These are synthetic Linux results;
Windows native, Android/JNI/device, performance and physical acceptance remain
separate.

## Windows runner dependency

`producer/picked_tests.rs` requires `--test-threads=1` for the giant-image suite.
The two-input permit is process-global; legacy producer and Android-copy tests
have different local serialization locks, and picked/URI tests use the same real
permit. The startup-saturation test also exercises the process-global four-worker
admission. `tools/ffi-test/core-unit.ps1` currently requests two threads. The four
picked/URI failures are consistent with that mismatch; cloud has not reproduced
the Windows failures or proved concurrency was their only cause.

Before Windows aggregate replay, the runner owner must:

1. Run the ownership suite with one test thread, preserving the real counters.
2. Reconcile its strict zero-ignored census with the intentional child probe.
   Admit only the exact known delegated child paired with its exact passing
   bounded wrapper and child-execution proof. Keep the raw ignored count and
   delegated-child count separate. Unknown ignored cases, missing/duplicate
   wrappers or proof, malformed/empty/failed census and missing required cases
   must continue to fail. Never blanket-accept ignored tests or run the quarantining
   child in the aggregate process.
3. Add runner regressions for those admission/refusal cases, then replay the
   seven exact producer cases and both verification integration cases using the
   native Windows artifacts before the broader single-thread native suite.

Runner and runner-fixture edits were not included in the granted producer source
partition. This branch is for reviewed Windows integration; cloud does not move
remote main or claim that the Windows aggregate is repaired from Linux evidence.
