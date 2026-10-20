# Reviewed Windows release integration — 2026-10-20

The accepted Windows branch
`c08078e7e7ccd77fd995679b1468977c5951b115` is merged into published release
main `d6ab2404b4e4d247462064d5ed9c7f6dc426ce6e`. Both histories are retained.
Only its three Rust leaves and dated native receipt are added; the current
release/MSI implementation is preserved. Independent Astra read-only comparison
confirms those four files remain byte-identical to the accepted branch.

The native change replaces unsupported whole-digest `LowerHex` formatting with
per-byte, leading-zero-preserving lowercase hex in the two Windows Paint evidence
callers. The private credential target helper is test-only. No dependency,
receipt bytes, authorization, credential target or application behavior changes.

The parent reports the Windows native/Android/bindings/goldens/alignment lane
passed after the earlier native receipt was written. Cloud did not repeat those
heavy commands or inspect the raw Windows logs. This report is compilation and
artifact evidence, not app-image, device, GPU, physical or product acceptance.

Windows then reported the 48-method assembler run as 39 passes, seven existing
POSIX-only skips and two failures. Both are fixture expectation mismatches. The
retained Windows read lease correctly rejects the injected second writer before
the POSIX-style mutation can happen. `test_source_mutation_while_streaming_is_detected`
now requires a native noncreating/nontruncating write-open to fail with Win32
sharing error 32 under that exact lease, plus the Python permission refusal,
unchanged source bytes, one mutation attempt and no COMPLETE. The same file
admits a writer before acquisition and after retirement. POSIX still requires
exact `input_changed` and proves the attempted bytes actually changed.
No test is skipped and no production lease or mutation check is weakened.

The second failure is `test_output_containment_and_missing_parent_refuse`:
opening the deliberately absent output parent yields exact
`Rejected("path_lease_unavailable")` on Windows and `FileNotFoundError` on POSIX.
The fixture now requires the correct exception on each platform while retaining
the root/overlap refusals. It also proves the parent and output remain absent,
the original source bytes/metadata are unchanged, and no fixture paths appear or
disappear. Production path, sharing, reparse and mutation protections are unchanged.

Fresh Linux replay runs 48 methods: 46 pass and the two actual-Windows metadata
witnesses are skipped. Both repaired fixtures execute their POSIX paths here;
their Windows assertions remain unexecuted in cloud. The three offline policy
checks pass. Independent Astra review covers the final fixture source.

Replay all 48 methods on Windows and retain any new failure trace. The two
deliberate fixture repairs do not establish that replay passed. The local task
retains real build, packaged image, S23 and runtime validation ownership.

The [machine-readable receipt](evidence/WINDOWS_RELEASE_INTEGRATION_20261020.json)
binds the exact native files, final fixture and cloud test log. No cloud heavy
build, binding regeneration, installer, provider, capture or device action ran.
