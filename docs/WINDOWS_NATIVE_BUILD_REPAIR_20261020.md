# Windows native build repair — 2026-10-20

The first current-source Windows Desktop build at main
`f3189ed21a57fced2b0025e31636dc62c1d974f2` reached the real Windows-only
Paint evidence code and failed with two E0277 errors. The pinned sha2 0.11
digest array does not implement `LowerHex`; both callers used
`format!("{:x}", Sha256::digest(...))`.

`session/remote_compatibility.rs` and `session/remote_drawing.rs` now encode
each digest byte with `02x` and collect the string. The receipt bytes and
serialized context tuple are unchanged. Encoding remains 64 lowercase hex
characters with leading zeros; evidence comparisons, currentness checks,
ownership and authorization are unchanged. No dependency changed.

The Windows credential backend's private `target()` helper is used only by a
unit test and now has `#[cfg(test)]`. Production continues to use `target_for()`;
credential targets, persistence and secret handling are unchanged. No actual
credential operation was performed for this repair.

Implementation was explicitly delegated to Sol 6.1/xhigh and the actual
three-file diff independently reviewed by Astra/high, with no actionable
findings. Tool interfaces do not expose additional runtime model metadata.

Validation on the authorized Windows laptop:

- Three-file `rustfmt +1.99.0 --check --edition 2024` and `git diff --check` pass.
- `build.ps1 build-ffi -TimeoutSeconds 600` reran all required license gates
  and its Windows native phase passed. Cargo compiled the repaired Windows
  code and produced the native libraries/helpers/binding tools. The bounded
  phase finished in 54.708 seconds, parent exit 0, with completed streams and
  confirmed process-tree cleanup. Its only remaining owned process was the
  verified MSVC telemetry helper, cleaned by the existing compiler policy.
- Android native compilation and subsequent golden/binding phases were still
  running when this source receipt was written; they are not passed here.

This is Windows compilation evidence. It does not establish a complete app
image, launcher/GPU presentation, MSI, device installation, physical pen,
performance, provider quality, runtime admission or owner/release acceptance.
The cloud coordinator continues to own the separate release/MSI sources.
