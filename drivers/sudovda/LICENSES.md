# SudoVDA source provenance

Upstream: https://github.com/SudoMaker/SudoVDA

Pinned source commit: `a4b09fa2aa731a964d0cb5d139cb1e6240e4da12`.
The task explicitly requests a source commit. This is not a stable binary release;
its own protocol header reports version 0.2.1 with `TestBuild=true`.

SudoMaker offers its own changes under MIT, CC0 or public domain in the retained
upstream README. This project selects CC0 for those contributions. Upstream has
no standalone LICENSE file. Microsoft's underlying IddCx sample is MS-PL; the
complete Microsoft license is retained in LICENSE-MS-PL.txt and copyright and
attribution notices remain unchanged in the vendored files. Source distribution
of those Microsoft portions remains under MS-PL, as required by that license.
This separate driver is the documented D11 license exception, never linked into
the proprietary Rust probe or main application.

**Incomplete vendor; build and distribution blocked.** `upstream.json` binds the
11 retained files byte-for-byte. The upstream EDID header and binary are held
outside tracked source, with their hashes and original paths in the manifest.
Upstream credits the EDID data to
[akatrevorjay/edid-generator](https://github.com/akatrevorjay/edid-generator), whose
repository is GPL-3.0. Use of that generator alone does not establish its output's
license. No separate grant for the inherited blob has been established here.
Do not assert that the data is GPL or copy generator code into this project.
Resolve the output's provenance or replace it with independently permitted data
before completing the vendor, compiling or distributing the driver. The current
fail-closed source gate intentionally prevents a partial vendor from building.

Microsoft license source retrieved on 2026-10-02:
https://raw.githubusercontent.com/microsoft/Windows-driver-samples/main/LICENSE

There are no bundled driver binaries, catalogs, certificates, keys, WDK tools or
SDK packages. WDK/SDK NuGet packages stay in the ignored local development cache.
DevGen is a development test tool and must never be redistributed with the app.
