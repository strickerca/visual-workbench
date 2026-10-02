# Giant image measurement

Run `python tools/bench/image/fetch_vips.py` through `Invoke-VwProcess` (180-second
limit) to populate the official portable tool pin. It verifies the release archive
size/SHA-256 and CLI bytes, checks ZIP path containment and never installs system
components. Existing altered or incomplete caches fail for inspection.

`build.ps1 hil-test image-pc` measures the hash-bound 200 MP JPEG using the verified
local libvips tool pin. Three sequential observations each force a full decode
(`avg`), create a 2040x1530 thumbnail and create a 256px DeepZoom pyramid. Two
workers and 128 MiB libvips cache are used; the cache cap is not a process memory
limit. Each command has a 180-second limit; the outer process tree has 600 seconds.

Timing includes process startup and output I/O. The OS file cache is never flushed,
so the first observation is not a cold-disk claim. Windows process handles remain
open until `GetProcessMemoryInfo` captures the OS peak working set. Pyramid tiles
are JPEG Q90 at all levels, rather than the eventual product's mixed lossless
WebP/JPEG encoding. Temporary derivatives are written outside Git and disposed;
the receipt keeps tile counts and cleanup results. No screenshots are created.

See `../image-android` for S23 measurements. These are decoder and CLI measurements,
not an implemented viewport renderer or integrated import latency proof.
