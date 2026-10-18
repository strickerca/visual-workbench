# Giant foreign retirement correction — source checkpoint

Fix code `181e4a40968583ea5511f0aad335b07f80a180d9`, composed with the accepted editing correction at `2b8d159db1eab49225b1d1199f2867ba59da17b9`.
Main is the exact accepted editing checkpoint a1ad07f. This giant correction is
held for focused checks and Astra recheck. No compiler/test was started while the
image owner holds the heavy slot. Formatting, whitespace and source secret scans
pass; the new regression cases are written only.

G1/P2 exposed a cross-owner lifetime error: a quarantined Java producer retained
its 80 MiB charge and slot, while JNI let the exact PreparedSource go. A later
native input close recursively deleted Java output/marker and invalidated retry.
The bounded native registry now pins that exact source before Java invocation.
Every unproven exit keeps it. The fixed pinned Java runtime alone confirms the
operation after files, charge and slot retire, including an explicit quarantine
retry. A read-only receipt gates native publication even if Java returns zero.
Failed attachment/notification remains conservatively held rather than silently
credited; the two native input permits and fixed two-entry registry bound it.

The seven changed source files and hashes are recorded in
`evidence/GIANT_FOREIGN_RETIREMENT_CORRECTION_20261018.json`. Portable tests close
an actual native input under seven unproven exits, assert the original and output
marker remain with 84 MiB charged, then simulate exact foreign retry and assert
zero charge/final deletion. A second test covers exact/bounded pin identity.
The Android case covers actual Java quarantine membership and cleanup retry;
actual cross-JNI cleanup remains a separate runtime gate.

After the image owner's slot handoff, run the focused native cases and strict
Linux/Android Clippy, build the changed native library, compile actual Android
main/instrumentation and run the composed unit suites. Verify `retiredNative`
`(J)I` against the new arm64 symbol. No UniFFI contract changed, so per-commit
binding regeneration is unnecessary. The prior 547 native / 944 JVM producer
receipt and 949 JVM editing receipt retain their original source bindings and do
not count as validation of this fix. Submit focused source/results to Astra;
G1 stays open until its verdict. No physical/codec/GPU/performance gate passes.

The image worker's picked.rs/picked_tests.rs/input.rs grant is unchanged and none
of those files was edited. Capture/Air Actions/T5.01 accepted leaves and initial
picker caller work are preserved on the separate uncompiled a5b683d draft.
Old worktrees, archives, failed cleanup receipts, providers and devices are untouched.
