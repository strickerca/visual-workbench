"""Bind pairing harness execution to the centrally gated source/binary build."""
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
RECEIPT = ROOT / '.local/pair-cli-build.json'
START = ROOT / '.local/pair-cli-build-start.json'
BINARIES = ['target/debug/pair-cli.exe', 'target/aarch64-linux-android/debug/pair-cli']


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def sources():
    files = {'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml',
             'deny.toml', 'build.ps1', 'contracts/vw_protocol.proto', 'contracts/storage.sql',
             'tools/pair-cli/build_receipt.py'}
    folders = [f'core/crates/vw-{crate}' for crate in ['geom', 'proto', 'model', 'ops', 'store', 'net']]
    folders += ['host-win/crates/vw-host-win', 'tools/pair-cli/src']
    files.add('tools/pair-cli/Cargo.toml')
    for folder in folders:
        for path in (ROOT / folder).rglob('*'):
            if path.is_file() and path.suffix in {'.rs', '.toml', '.sql', '.proto'}:
                if path.is_symlink() or not path.resolve().is_relative_to(ROOT):
                    raise ValueError('Unexpected source path')
                files.add(path.relative_to(ROOT).as_posix())
    return {name: digest(ROOT / name) for name in sorted(files)}


def binding():
    return {
        'schema': 1,
        'source_sha256': sources(),
        'binary_sha256': {name: digest(ROOT / name) for name in BINARIES},
        'targets': ['x86_64-pc-windows-msvc', 'aarch64-linux-android'],
        'android_api': 29,
        'profile': 'debug',
    }


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in {'begin', 'record', 'check'}:
        raise ValueError('Use begin, record or check')
    if sys.argv[1] == 'begin':
        RECEIPT.parent.mkdir(parents=True, exist_ok=True)
        # A failed replacement build must never leave an old successful receipt.
        RECEIPT.unlink(missing_ok=True)
        START.write_text(json.dumps({
            'schema': 1,
            'snapshot_id': uuid.uuid4().hex,
            'started_at_utc': datetime.now(timezone.utc).isoformat(),
            'source_sha256': sources(),
        }, indent=2) + '\n', encoding='utf-8', newline='\n')
        print('Pairing source snapshot captured before both target builds')
        return
    current = binding()
    if sys.argv[1] == 'record':
        start = json.loads(START.read_text(encoding='utf-8'))
        if start['source_sha256'] != current['source_sha256']:
            raise ValueError('Pairing source changed between target builds; rebuild both')
        current['source_snapshot_id'] = start['snapshot_id']
        current['source_snapshot_started_at_utc'] = start['started_at_utc']
        current['source_snapshot_captured_before_build'] = True
        RECEIPT.write_text(json.dumps(current, indent=2) + '\n', encoding='utf-8', newline='\n')
    else:
        recorded = json.loads(RECEIPT.read_text(encoding='utf-8'))
        if (not recorded.get('source_snapshot_captured_before_build') or
                any(recorded.get(key) != value for key, value in current.items())):
            raise ValueError('Source or binaries changed; rebuild through build.ps1 build-pairing')
    print('Pairing source and two executable bindings: PASS')


if __name__ == '__main__':
    main()
