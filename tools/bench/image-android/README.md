# S23 image diagnostic

Build with `build.ps1 build-image-android`, then `build.ps1 hil-test image-android`.
The runner verifies current APK/source bindings and synthetic fixture hashes,
selects only SM-S918U, installs this separate diagnostic and uses UUID-owned files.
Every mode starts a fresh process; decode memory therefore excludes HEIF generation
buffers. It checks foreground, polls recorded phases and stops only this app on
failure or completion. Logs/receipts are local and device identifiers are redacted.

JPEG measures three 1024px regions and an inSampleSize=8 full-image decode, three
observations each. Timings include region decoder creation. HEIF uses a fixed YUV
gradient and HeifWriter 1.1.0, grid encoding, Q95. The first buffer-mode experiment
failed with OutOfMemoryError on the 512 MiB Java heap: two full direct YUV buffers
plus the Java input total about 899 MB, and the initial guard did not account for
all three against Java heap limits. That failure and its APK binding are retained.
The current generator uses a 2040x1530 RGB bitmap (12.5 MB) scaled by HeifWriter's
normalized grid texture coordinates to 16320x12240. It has 200 MP output dimensions,
not 200 MP independent source detail. The guard reserves two pattern buffers plus
512 MiB system headroom and 64 MiB Java headroom. Generation has a
120-second writer stop limit and a 180-second external deadline. Output is pulled
only after a successful receipt and hash check.

ImageDecoder measures 2040x1530 software target decoding three times and checks
three gradient sample positions to detect empty output, flips or wrong scaling. Full-sized
RGBA needs about 762 MiB: the guard computes 16320*12240*4 =
799027200 bytes and permits no more than min(75% Java maximum heap, one-third of
available system memory). A guard skip is explicitly recorded, never a full-decode
success. PSS baseline, sampled peak (100ms) and bitmap allocations are recorded;
sampling can miss transient spikes and is not GPU memory measurement.

The diagnostic requests largeHeap to explore platform behavior. This does not
change the product's 700 MB PSS requirement. No private photos, hostile corpus,
network permissions, screenshots or physical pen actions are involved. The pulled
synthetic HEIF is a reusable ignored fixture, not a verification screenshot.
