"""Bind both native binaries to the transport sources and gated build inputs."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
RECEIPT = ROOT / '.local/transport-build.json'


def bindings():
    names = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml',
             'deny.toml', 'build.ps1', 'tools/check_setup.py',
             'tools/bench/transport/Cargo.toml', 'tools/bench/transport/build_receipt.py',
             'target/release/transport-bench.exe',
             'target/aarch64-linux-android/release/transport-bench']
    paths = [ROOT / name for name in names]
    paths.extend(sorted((ROOT / 'tools/bench/transport/src').glob('*.rs')))
    return {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def main():
    current = bindings()
    if sys.argv[1:] == ['record']:
        RECEIPT.parent.mkdir(exist_ok=True)
        RECEIPT.write_text(json.dumps({'schema': 1, 'sha256': current}, indent=2)+'\n',
                           encoding='utf-8', newline='\n')
        print('Transport gated-build source and binary binding recorded')
    elif sys.argv[1:] == ['check']:
        if json.loads(RECEIPT.read_text(encoding='utf-8'))['sha256'] != current:
            raise SystemExit('Transport build differs from source: rerun build.ps1 build-transport')
        print('Transport gated-build binding verified')
    else:
        raise SystemExit('Usage: build_receipt.py record|check')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError):
        # Missing/stale inputs may contain private absolute paths in exceptions.
        raise SystemExit('Transport build receipt unavailable or invalid; gated rebuild required') from None
