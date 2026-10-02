"""Check vendored byte provenance; --build fails until the EDID license gap closes."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
COMMIT = 'a4b09fa2aa731a964d0cb5d139cb1e6240e4da12'

def check(root=ROOT, build=False):
    manifest = json.loads((root / 'upstream.json').read_text(encoding='utf-8'))
    if manifest.get('commit') != COMMIT or manifest.get('schema') != 1:
        raise ValueError('Unexpected upstream source binding')
    vendor = (root / 'vendor').resolve()
    listed = set()
    for name, digest in manifest['files'].items():
        relative = Path(name)
        path = (vendor / relative).resolve()
        if relative.is_absolute() or '..' in relative.parts or not path.is_relative_to(vendor):
            raise ValueError('Unsafe vendor path')
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError('Vendor hash mismatch: ' + name)
        listed.add(path)
    if listed != {p.resolve() for p in vendor.rglob('*') if p.is_file()}:
        raise ValueError('Unlisted vendor file')
    if build and (not manifest.get('complete_vendor') or manifest.get('held_for_license_provenance')):
        raise ValueError('Driver build blocked: EDID output licensing/provenance is unresolved')
    return len(listed)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build', action='store_true')
    arguments = parser.parse_args()
    try:
        print('SudoVDA source hashes verified:', check(build=arguments.build))
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, str(error) + '\n')
