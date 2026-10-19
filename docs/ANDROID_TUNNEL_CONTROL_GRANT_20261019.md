# F continuation: explicit owner Android control

Parent dispatch to existing F `01a14eeb-8086-76af-9670-dd5ffa5eb032`, after its
current compile/standalone-harness repair. Do not start another owner. Its
mirror/capture source is not TUNNEL-005 control: Windows launch currently fixes
`--no-control` and there is no enabling route. This is remaining implementation,
not a request for live device use or a statement of product acceptance.

Exclusive existing F `host-win/crates/vw-remote-host/src/android_tunnel/**`,
matching tests and `docs/ANDROID_TUNNEL_CONTROL_HANDOFF_20261019.md`. Root keeps
Cargo/lock/features, native/shared FFI, actual consent/UI, Main/controllers,
runtime/license registration and capture/stream guard hooks. Preserve current
view-only, paired capture, exact device selection and retained retirement work.

Implement and test these fixed boundary decisions:

- Closed `ViewOnly` / `OwnerControl` interaction mode. ViewOnly remains the
  ordinary default and always passes --no-control. OwnerControl omits only that
  restriction after its exact native permit is consumed. No arbitrary argv,
  shell/adb input command or silently enabled control when a mirror opens.
- A separate opaque `OwnerControlPermit`, distinct from license/runtime
  admission. Its root-only minting boundary binds the actual owner-confirmed
  target descriptor to the current session UUID/generation, opaque selected
  device token and retained adb/runtime file identities, with a fresh one-shot
  nonce. It expires and cannot be replayed after replacement/disconnect/revoke.
  No public approval boolean, parsed JSON/hash or paired network message mints
  it. Return the minimal root mint/consume API; do not implement fake root
  approval or an always-true provider. Tests may use an explicitly test-only
  synthetic issuer. Missing/mismatched/expired/consumed permits refuse before
  launching a controlling process.
- Root will display the actual selected phone and interaction mode, obtain
  explicit local owner confirmation and mint the matching permit. USB debug
  authorization, external installed adb and runtime/license admission remain
  separate prerequisites. A fresh device census and retained identity checks
  fence launch. No PATH/bundled-adb fallback, root, tcpip/connect or server reset.
- If changing an existing mirror's mode, stop and join the exact old Job/IO
  before launching a replacement. Unresolved retirement retains the owner and
  blocks the change. Update actual owned-mirror PID/creation/HWND registration
  for the replacement; old capture permissions cannot treat it as an unowned
  window. Source-loop checks still apply before capture/readback/publication.
- Return actual read-only interaction/retirement state. A permit or launched
  process is not evidence that a phone input event was delivered. Session
  replacement and explicit revoke consume/seal the permit and retire owned
  processes without touching a pre-existing adb server or another device.

Synthetic tests: default view-only argv; exact controlling argv only after
permit consumption; absent/wrong target/generation/device/binary/expired/reused
permit; retirement-pending mode switch; old mirror census rejection; cancel
before launch and during replacement. No real adb, scrcpy, DLL, GUI/input/device,
provider, security/identity, signing/deployment or driver execution. Commit
logical source/tests; report actual compiled cases separately from Windows and
hardware checks. Root still owes normal UI/FFI/minting/caller integration before
the feature can be called implemented end to end.

Root cheap compile note: exact7533f7c0 fails with the requested default-feature
roxmltree0.21.1 dependency because ParsingOptions omits entity_resolver. An
explicit `entity_resolver: None` preserves deny-DTD/no-external-resolution
behavior; four synthetic tests pass only in the temporary copy with that fix.
Reconcile this exact pin/features/source receipt with earlier narrow review
claims; do not label the published leaf compiled from the patched-copy result.
The separate Windows standalone-harness Pins import repair remains F-owned.
