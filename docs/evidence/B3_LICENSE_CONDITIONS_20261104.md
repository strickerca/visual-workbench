# B3 license admission conditions — 2026-11-04

These are the D22 conditions for the two B3 admissions. Downloads went into
fresh private directories under `.local/downloads/` and are not in Git. Nothing
was extracted, executed or loaded.

## scrcpy v4.1 Android tunnel runtime

- **Archive:** `scrcpy-win64-v4.1.zip` from the official GitHub release,
  11,305,298 bytes, SHA-256
  `5b12172b3264b2889f4583ee64752ce832e29bc8b1089dca81093459697165db`. This
  matches `tools/android-tunnel/pinned-runtime-inventory.json`.
- **DLL scan:** a read-only scan of the six catalog DLLs, read from the zip in
  memory, found:

  | DLL | SHA-256 matches pin | Embedded license | GPL/version3/nonfree configure flags |
  | --- | --- | --- | --- |
  | avcodec-62.dll | yes | LGPL version 2.1 or later | none |
  | avformat-62.dll | yes | LGPL version 2.1 or later | none |
  | avutil-60.dll | yes | LGPL version 2.1 or later | none |
  | swresample-6.dll | yes | LGPL version 2.1 or later | none |
  | libusb-1.0.dll | yes | no embedded string (upstream LGPL-2.1+) | n/a |
  | SDL3.dll | yes | no embedded string (upstream zlib) | n/a |

- **Result:** the D22 condition is met. The FFmpeg DLLs are LGPL builds, and
  there is no GPL or nonfree code.
- **Still to do:** compile the approved catalog into
  `android_tunnel_root.rs` (`COMPILED_APPROVED_CATALOG`), add the notices, then
  run the device journeys (T5.03).

## WiX Toolset 3.14.1

- **Archive:** `wix314-binaries.zip` from the official `wixtoolset/wix3`
  release `wix3141rtm`, 41,297,555 bytes (matches the release listing). The
  recorded SHA-256 is
  `6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31`; GitHub
  publishes no digest.
- **Result:** the D22 condition is met. This is the admitted WiX input for
  T5.06. The cached WiX 3.11 is not used.
- **Still to do:** the inventory, compatibility and license-review evidence
  that `tools/release/MSI_INPUTS.md` requires, followed by MSI construction.
