# Private picked-input root composition

Code `04a913e3e36d4b6e18bf4b8b3d5a286e3ecf8b7a` on cloud/giant-picked-input-integration-20261018 composes exact
accepted ba338ad over reviewed main30b77615. The three leaf files are byte-identical
to the accepted source. Root adds only `mod picked` and the exact private exports
PendingInputAdmission, PickedOriginal, reserve_picked_input, with unused-import
allowance scoped to that temporary private export. No public intake is activated.

Root must call reserve_picked_input with the existing shared pool BEFORE opening
its actual File or creating TempDir, then attach synchronously. The selected,
prepared, producer and failed cleanup owners share the SAME existing input permit/
4MiB; no duplicate slot/pool is created. After prepared delivery, picked close/drop
cannot close the independent prepared handle. No foreign completed directory,
approval flag or external file descriptor is admitted by this composition.

PI1 is closed within Astra's narrow accepted scope: shared control retires its
sole startup sender without requiring a completed future to be polled/dropped.
Actual file cleanup/quarantine retains input permit/4MiB, startup admission stays
with a running job, and abandoned results cannot credit live resources. Close is
NOT an OS-thread join. The Linux eight-future test observes eventual zero actual
startup workers under bounded waits; do not claim an instantaneous thread cap,
strict peak memory, Android runtime execution or hardware acceptance from it.

The composed batch passes54 parent cases:17 picked, four G1 bridge and33 existing
producer/cache/publication cases. One ignored child entrypoint is actually invoked
by its bounded wrapper, which requires its exact name and one executed/passed
case. Linux all-target and arm64 library Clippy pass with warnings denied. First
attempts pass. Source/hash/log receipts are in
`evidence/GIANT_PICKED_INPUT_COMPOSED_20261018.json` and its excerpts.

No dependency/public UniFFI change or binding generation. No repeated app batch:
actual native packaging and public callers belong to the next coherent accepted
caller phase. Android Air Actions, Desktop capture, native verification and shared
diagnostics grants are untouched. New lineage88946ba/8bf738a remains independent
pending review; diagnostics8c90d0a is fetched only pending its source verdict/tests.
No provider, device, signing/deployment, SDK or old-worktree operation occurred.
