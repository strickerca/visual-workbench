# F tunnel control handoff — 2026-10-19

This leaf implements the producer-side TUNNEL-005 boundary. An Android tunnel
opens in `InteractionMode::ViewOnly`, and the fixed scrcpy argv includes
`--no-control`. The only other mode is `OwnerControl`: after a matching,
unexpired, one-shot permit is consumed, its fixed argv omits only
`--no-control`. There is no input-command or arbitrary-argv route.

The root call sequence is:

1. Complete the existing exact-byte runtime/license admission, acquire an
   owner-selected external platform-tools `adb.exe` and USB selection, then
   call `AndroidTunnelOwner::open(...)` with a fresh session UUID/generation.
2. Call `control_challenge(&cancel)`. It rechecks the selected USB device,
   live owned mirror, and pinned binary identity. Display its
   `challenge.target()` model and transport ID, along with the requested
   control mode, and receive a fresh explicit **local owner** confirmation of
   that displayed target.
3. Only that root confirmation handler may call crate-private
   `OwnerControlPermit::mint_from_root_confirmation(&challenge,
   confirmed_target, lifetime)` with lifetime at most 30 seconds. This
   constructor is intentionally unavailable to peer/project messages and
   has no approval bool, JSON receipt, or hash-string shortcut. The permit
   binds the session, generation, opaque selected-device token, retained
   adb/runtime file identity, fresh random nonce, mirror revision, and
   confirmed target.
4. Pass the permit to `enable_control(Some(&mut permit), &cancel)`. The producer
   repeats the census/identity check, consumes the permit, joins active IO and
   retires the exact old mirror Job, checks the selection and pins again, then
   launches the replacement. A mismatch, expiry, or replay refuses before
   controlling launch and burns the challenge. After an uncertain retirement
   or failed replacement the owner is sealed; call `close()` and, if needed,
   `retry_retirements()` until the old Job and IO are proved retired.
5. Capture callers must retain `mirror_revision()` from the current owner and
   pass it into `check_source(source, revision)` before capture/readback or
   publication. Reusing an old mirror revision returns `Stale`; the native
   window probe follows the new owned PID/HWND registration after replacement.
   `interaction_state()` reports view-only, owner-control, retirement-pending,
   or closed; it does not assert that a phone input event was delivered.
   `revoke_control()` closes the exact owned mirror/IO. A return to viewing
   requires a new owner session and challenge.

The root still owns the native/shared FFI, actual UI confirmation, session
controller wiring, runtime/license registration, and source-loop guard hooks.
This leaf alone is not an end-to-end control feature. No phone input event,
real adb/scrcpy/Windows GUI, or hardware control path has been exercised.

The earlier isolated root build of F7533f7c reported E0063 under default-feature
`roxmltree 0.21.1`. This leaf supplies `entity_resolver: None`, retaining
denied DTD parsing. A Linux isolated harness using the actual pinned `ring`,
`png`, `blake3` and default-feature `roxmltree` dependencies compiled and passed
all five synthetic test functions; clippy with warnings denied passed there.
The Windows tunnel, its tests and exact `process/windows.rs` Pins source
typechecked in a separate source-only cross-target harness with **non-runtime
ring and host-error stand-ins**.
An actual-ring Windows cross-check stopped in its dependency build because this
Linux host lacks MSVC `lib.exe`. Neither result is a Windows executable test.

The source-lane standalone integration test now wires the real Pins source on
Windows and the tunnel's module paths resolve from both test and library roots.
Root must add `roxmltree = "=0.21.1"` to `vw-remote-host/Cargo.toml` and update
the root lock if needed, then register `android_tunnel` in the host library only
after the exact runtime/license gate and real owner-confirmation caller exist.
The current host manifest does not declare `roxmltree`, so a normal host-crate
test/build is not yet evidence for this leaf. Root should rerun the standalone
test, a native Windows build and actual UI/device validation after integration.
