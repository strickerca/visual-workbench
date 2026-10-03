# Native WebP notices

Retrieved 2026-10-02 from the official crates.io source archive for
`libwebp-sys = 0.14.4` (released 2026-04-29; declared MIT).
The archive's `.cargo_vcs_info.json` binds commit
`3a40281fd97537b693174acb7375d64d3b471a56` in
https://github.com/NoXF/libwebp-sys .

- Package/release: https://crates.io/crates/libwebp-sys/0.14.4
- Exact source archive: https://crates.io/api/v1/crates/libwebp-sys/0.14.4/download
- `vendor/src/enc/vp8i_enc.h` declares encoder **1.6.0**.
- `libwebp-COPYING` and `libwebp-PATENTS` are exact text from that archive's
  `vendor/COPYING` and `vendor/PATENTS`: BSD-3-Clause and additional patent grant.
- No system library, bindgen, threading or parallel C compilation features are
  enabled. The upstream build still selects architecture-specific codec kernels;
  no claim is made that lossy WebP bytes match across architectures. PNG and
  shared text layouts are the committed cross-platform byte golden boundary.

The earlier considered `webp=0.3.1` wrapper is not used: it constrains the native
dependency to the older `libwebp-sys 0.9` line. The owned FFI adapter checks input,
errors and native output lifetime directly against current pinned bindings.
