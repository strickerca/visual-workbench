# Pinned Windows producer source handoff

This crate recovers the 11 archived R3 files after byte count and SHA-256 checks
against the source-candidates root MANIFEST.json. The source recovery receipt is
in `/tmp/vw-assets-validation/giant-producer-next/windows/archive-verification.json`.
No runtime, helper executable, provider, installation or license acceptance was
performed. D15 remains Windows-only dynamic replaceable libvips 8.18.7 with the
unchanged runtime-pins.json inventory; Android never loads this runtime.

The coordinator integrates this private API from its retained native worker:

```rust,ignore
build_budgeted(helper: &LockedHelper, runtime: &LockedRuntime,
    input: &Path, destination: &Path, request: &BuildRequest,
    budget: &vw_stream::SessionBudget, cancel: &Cancellation)
    -> Result<BudgetedManifest>
```

The root workspace already includes `host-win/crates/*`. The coordinator adds
`vw-assets-win = { path = "../../../host-win/crates/vw-assets-win", version = "=0.1.0" }`
under the Windows target dependencies in core/crates/vw-ffi/Cargo.toml, then owns
the coherent root Cargo.lock update and all build/check/lint execution. Recovered
third-party dependencies keep existing workspace lockfile versions. This crate
adds a path dependency on the existing vw-stream crate.

`Cancellation::from_probe(Arc<dyn Fn() -> bool + Send + Sync>)` observes the root
cancellation token and prepared-input closed gate without a watcher thread. A
true result cancels; local cancel() also seals the operation. The probe must be
nonblocking and retain its captured owners. The synchronous call and its resource
owners must remain on the retained native worker until actual completion even
if the requesting async future is abandoned.

All production reservations come from the supplied session. A 2 MiB parent work
envelope precedes local allocations. Before CreateProcessW the parent reserves
WORKER_MEMORY_BYTES (256 MiB); the suspended child is immediately put in an owning
guard, attached to its unique Job, and only then resumed. The Job enforces total
and per-process 256 MiB commit caps and one active process. The charge remains in
that guard through actual process termination and Job active-process count zero,
including assignment failure, cancellation and timeout. Failure to prove worker
retirement keeps the guard joining and retains the charge; shutdown responsiveness
is unmeasured. The child creates no independent SessionBudget: its entire process
is covered by the parent's allowance. libvips uses two threads and a 64 MiB cache.

After child retirement, a 24 MiB manifest envelope precedes bounded raw read and
Manifest decoding. BudgetedManifest retains this reservation, with Manifest
declared before MemoryCharge for correct destruction order. Use manifest() and
charge(), or move both into one payload owner with into_parts() before fallible
work; do not clone the Manifest without separately reserving the copy. Exact
encoded tile length is reserved before allocation; locked reads use a stack
sentinel and never grow the Vec. A separate 32 MiB decode reservation precedes
codec entry and remains through decoded-pixel destruction. Memory exhaustion is
Error::Memory { estimated, budget }; errors contain no source bytes or paths.

Root full source/profile/depth/header admission and explicit untagged-sRGB consent
remain prerequisites. The crate's recovered admit_input is a narrow codec-entry
guard. Publication still uses sibling private staging and MoveFileExW flags zero,
after complete inventory/hash/codec verification; it never replaces an existing
destination or rewrites the immutable source. Root must retain its source identity
and workflow checks around this call and cache installation.

The initial source handoff used only formatting/hash/source inspection. The
subsequent producer worker validation passes14 portable cases and strict
all-target Linux Clippy. Windows-specific code/execution and12 Windows-only
cases (including2 opt-in packaged runtime cases) remain unverified. Windows
target checking stops at missing lib.exe. See
`core/crates/vw-assets/GIANT_APPROVED_PRODUCER_HANDOFF_20261018.md` for the exact
final source-bound scope and root admission prerequisites. The reduced worker
cap still awaits runtime measurement; these checks do not prove process RSS,
codec peaks, timing, quality, OS teardown or target performance acceptance.
