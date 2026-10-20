# Unadmitted notice proposal disposition — 2026-10-20

All 125 files formerly under `third_party/notices/assisted-selection/` and
`third_party/notices/scrcpy/` are preserved here byte for byte. They are prepared
source-license and attribution candidates, outside the packaged `third_party`
tree. They have not been registered in `third_party/LICENSES`.

| Former directory | Preserved directory | Files |
| --- | --- | ---: |
| `third_party/notices/assisted-selection/` | [`notices/assisted-selection/`](notices/assisted-selection/) | 118 |
| `third_party/notices/scrcpy/` | [`notices/scrcpy/`](notices/scrcpy/) | 7 |

The 118 assisted-selection files comprise 107 nonempty wheel notice candidates,
seven sdist license texts and four pinned upstream license texts. The seven
scrcpy files comprise six source-license texts and the original `SOURCES.json`.
The complete set totals **1,428,759 bytes**.

[`disposition.json`](disposition.json) lists every former source path, present
destination path, byte count and SHA-256, with counts for each group. It also
records the unchanged identities of five original census, source, inventory and
proposal JSON files under `tools/`. Those records keep their original paths and
bytes so their existing hashes and historical receipts remain meaningful.

Paths in `notice-sources.json`, `component-census.json` and the two root manifest
proposal files remain historical or proposed package paths. They are not current
filesystem locations or permission to copy candidates into a distribution. The
explicit resolver in
[`notice_disposition.py`](../../tools/assisted-selection/license-audit/notice_disposition.py)
maps only listed former notice paths to this proposal tree. For example, the
scrcpy proposal entry `notices/scrcpy/SDL-3.4.12-LICENSE.txt` denotes a proposed
`third_party` destination; its preserved text now resolves to
`docs/license-proposals/notices/scrcpy/SDL-3.4.12-LICENSE.txt`.
The assisted audit and scrcpy notice tests use this map. The audit continues to
compare unchanged source/census records, including private sdist notice copies,
against the exact preserved bytes.

The dated [assisted license census](../ASSISTED_LICENSE_CLOSURE_20261019.md),
[scrcpy handoff](../ANDROID_TUNNEL_RUNTIME_HANDOFF_20261019.md) and earlier receipts
describe their original locations and runs. They remain history; this disposition
does not rerun their full private artifact or DLL checks. New tooling bytes differ
from the earlier tooling, and old tooling hashes are not claimed to describe them.

The root packaging check covers files actually in `third_party`; it must continue
to refuse unlisted files there. This relocation changes only the placement of
unadmitted proposals. Model/checkpoint rights, TinyViT lineage, host/native
aggregate review, actual DLL configuration, dynamic replaceability, required
notices and Android/runtime admission remain open. The compiled tunnel catalog
and model gates remain closed. No archive, model, executable or runtime has been
promoted by this disposition.

Cheap offline verification from the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/assisted-selection/license-audit/notice_disposition.py
PYTHONDONTWRITEBYTECODE=1 python3 tools/assisted-selection/license-audit/audit.py --source-only
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/assisted-selection/license-audit -p 'test_*.py' -v
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/android-tunnel -p 'test_*.py' -v
python3 tools/check_setup.py all
```

The disposition verifier rehashes all 125 preserved files and all five unchanged
JSON records. It refuses missing, changed or extra proposal files, altered maps,
an admitted disposition, and restored candidate directories in `third_party`.
Both disposition and source-only audit outputs retain
`production_admitted=false`. The Python tests use local files and synthetic
archives; they execute no DLL, provider, model or device.

The fresh [verification receipt](verification-20261020.json) binds the completed
checks to the updated tooling and disposition map. It records the root license
register's unchanged identity separately from the prior historical receipts.
