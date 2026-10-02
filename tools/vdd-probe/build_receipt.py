"""Bind the diagnostic binary to the exact source and gated build inputs."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]

def bindings():
    names = ['Cargo.toml','Cargo.lock','rust-toolchain.toml','.cargo/config.toml','deny.toml',
             'build.ps1','tools/vdd-probe/Cargo.toml','tools/vdd-probe/build_receipt.py',
             'target/release/vdd-probe.exe']
    paths = [ROOT / name for name in names] + sorted((ROOT / 'tools/vdd-probe/src').glob('*.rs'))
    return {p.relative_to(ROOT).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}

if __name__ == '__main__':
    try:
        current = {'schema':1,'sha256':bindings()}
        receipt = ROOT / '.local/vdd-probe-build.json'
        if sys.argv[1:] == ['record']:
            receipt.write_text(json.dumps(current, indent=2)+'\n', encoding='utf-8')
        elif sys.argv[1:] == ['check']:
            if json.loads(receipt.read_text(encoding='utf-8')) != current:
                raise ValueError('Stale source')
        else:
            raise ValueError('Unknown operation')
        print('VDD probe build binding: PASS')
    except (OSError, ValueError, KeyError):
        raise SystemExit('Missing, invalid or stale probe build receipt; gated rebuild required') from None
