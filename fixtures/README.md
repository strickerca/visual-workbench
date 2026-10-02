# Import fixture corpus

All corpus pixels, vector geometry, text and intentionally malformed bytes are
first-party synthetic inputs. None is a verification screenshot or an owner's
photo. The files are under the project's proprietary license; no third-party
images are included. `manifest.json` lists every retained fixture's provenance,
purpose, byte count and SHA-256. The generated media stay in ignored
`fixtures/generated/t009-v2/`, including the reusable phone-generated HEIF.
Do not stage generated media or `fixtures-private/`.

Generate the 18 host fixtures with `fixtures/generate.py --output
fixtures/generated/<fresh-name> --vips <verified-vips.exe>`, through the bounded
`Invoke-VwProcess` runner. Python tool pins: NumPy 2.3.5 (BSD-3-Clause), Pillow
12.1.0 (HPND/PIL), ReportLab 4.4.10 (BSD-3-Clause), pypdf 6.8.0 (BSD-3-Clause).
These are development tools only. ReportLab uses invariant mode, fixed passwords
and standard PDF fonts. The generator uses PCG64 seed 20016320 plus named offsets;
JPEG is quality 95, 4:2:0. The manifest binds the generator and encoder versions.
Changing a tool version requires a new manifest, not silent hash replacement.

`images/`, `pdf/`, `svg/` and `malformed/` describe corpus categories. The physical
files share one generated directory so fixture hashes can be checked without
special search paths. Verify using `python fixtures/verify.py
fixtures/generated/t009-v2 fixtures/manifest.json`.

The HEIF is generated on the S23 by `tools/bench/image-android`, using AndroidX
HeifWriter 1.1.0 (Apache-2.0) and hardware encoding. Its pattern and actual output
hash are recorded. Its 2040x1530 source pattern is scaled across the 200 MP output
grid; it does not represent 200 MP of independent camera detail. Hardware HEIF container/codec output is not promised to be
byte-identical across runs or OS releases; only its input pattern is deterministic.

The libvips 8.18.7 Windows web build is a standalone LGPL-2.1 measurement tool in
ignored local storage, not linked into or shipped with the application. See
`docs/TOOLCHAIN.md` for source pins and licenses. Full native redistribution
review is still required before product integration.

Hostile files are written and hashed, never rendered by generation or verification.
Future importer tests must apply dimensions/resource limits, reject entity
expansion and external references, and isolate decoder failures. Public fixture
passwords `vw-fixture` and `vw-fixture-owner` are test data, not credentials.

No real owner photo has been supplied. A future private 200 MP camera fixture
must have GPS metadata removed before storage and must remain outside Git.
