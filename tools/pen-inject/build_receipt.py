"""Reuse only the exact Windows probe binaries produced by the gated build."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
RECEIPT = ROOT / '.local' / 'pen-inject-build.json'


def bindings():
    paths = [ROOT/name for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
             'deny.toml', 'build.ps1', 'tools/check_setup.py', 'tools/pen-inject/build_receipt.py')]
    for crate in ('core', 'harness', 'inject'):
        base = ROOT/'tools'/'pen-inject'/crate
        paths.append(base/'Cargo.toml')
        paths.extend(sorted((base/'src').rglob('*.rs')))
    paths.extend(ROOT/'target'/'debug'/name for name in ('pen-harness.exe', 'pen-inject.exe'))
    return {path.relative_to(ROOT).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}


if __name__ == '__main__':
    try:
        current = bindings()
        if sys.argv[1:] == ['record']:
            RECEIPT.parent.mkdir(exist_ok=True)
            RECEIPT.write_text(json.dumps({'schema': 1, 'sha256': current}, indent=2)+'\n', encoding='utf-8')
            print('Windows probe gated-build receipt written')
        elif sys.argv[1:] == ['check']:
            if json.loads(RECEIPT.read_text(encoding='utf-8')) != {'schema': 1, 'sha256': current}:
                raise ValueError('Probe source or binaries changed; rebuild required')
            print('Windows probe source/binary hashes match the gated build')
        else:
            raise ValueError('Expected record or check')
    except (OSError, ValueError, IndexError) as error:
        # Paths may identify the owner; never include exception text in the log.
        print('Windows probe build receipt unavailable or stale; gated rebuild required', file=sys.stderr)
        sys.exit(1)
