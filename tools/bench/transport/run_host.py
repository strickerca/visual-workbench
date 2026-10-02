"""Run fresh, bounded Windows loopback measurements; never label them phone evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]
EXE = ROOT / 'target/release/transport-bench.exe'


def validate(report, profile, kind):
    count, bulk = (1000, 256 * 1024 * 1024) if profile == 'full' else (10, 1024 * 1024)
    if report.get('completed') is not True or report.get('transport') != kind or report.get('profile') != profile:
        raise ValueError('Incomplete or mismatched benchmark report')
    stream = report['stream']
    if stream['bulk_bytes'] != bulk or stream['payloads_verified'] is not True or stream['bulk_elapsed_ms'] <= 0:
        raise ValueError('Unverified bulk transfer')
    if [row['bytes'] for row in stream['echo']] != [64, 4096, 1048576]:
        raise ValueError('Missing echo sizes')
    for row in stream['echo']:
        if row['rtt']['count'] != count or len(row['raw_rtt_ms']) != count:
            raise ValueError('Missing RTT samples')
    if kind == 'quic':
        dg = report['datagrams']
        if dg['sent'] != (1200 if profile == 'full' else 120) or dg['received'] + dg['missing_roundtrip_echoes'] != dg['sent']:
            raise ValueError('Invalid datagram census')


def run(kind, profile, root):
    folder = root / kind
    log = (root / (kind+'-server.log')).open('wb')
    server = subprocess.Popen([str(EXE), 'serve', kind, '127.0.0.1:0', str(folder), '1200'],
                              stdout=log, stderr=subprocess.STDOUT, cwd=ROOT)
    try:
        deadline = time.monotonic() + 10
        while not (folder/'ready.json').exists():
            if server.poll() is not None or time.monotonic() > deadline:
                raise ValueError('Server readiness failed; inspect owned server log')
            time.sleep(.1)
        ready = json.loads((folder/'ready.json').read_text())
        if ready['transport'] != kind or not 0 < ready['port'] < 65536:
            raise ValueError('Invalid server readiness')
        out = root/(kind+'-client.json')
        cert = str(folder/'server.der') if kind == 'quic' else '-'
        subprocess.run([str(EXE), 'run', kind, f"127.0.0.1:{ready['port']}", cert, str(out), profile],
                       cwd=ROOT, timeout=1210, check=True)
        server.wait(timeout=20)
        if server.returncode or not json.loads((folder/'complete.json').read_text()).get('completed'):
            raise ValueError('Server did not complete cleanly')
        report = json.loads(out.read_text())
        validate(report, profile, kind)
        print(f"{kind} loopback: verified; {report['stream']['bulk_mib_per_second']:.2f} MiB/s", flush=True)
        return {'transport': kind, 'report_sha256': hashlib.sha256(out.read_bytes()).hexdigest(),
                'echo': [{'bytes': row['bytes'], **row['rtt']} for row in report['stream']['echo']],
                'bulk_mib_per_second': report['stream']['bulk_mib_per_second'],
                'datagrams': {k: v for k, v in (report['datagrams'] or {}).items() if k != 'raw_rtt_ms'}}
    finally:
        if server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)
        log.close()


def main():
    subprocess.run([sys.executable, str(ROOT/'tools/bench/transport/build_receipt.py'), 'check'],
                   cwd=ROOT, timeout=30, check=True)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', choices=['smoke', 'full'], default='smoke')
    parser.add_argument('--kind', choices=['tcp', 'quic', 'both'], default='both')
    args = parser.parse_args()
    root = ROOT / '.local' / ('transport-host-'+uuid.uuid4().hex)
    root.mkdir()
    result = {'schema': 1, 'environment': 'Windows loopback only; no physical carrier acceptance',
              'profile': args.profile, 'binary_sha256': hashlib.sha256(EXE.read_bytes()).hexdigest(), 'runs': []}
    try:
        for kind in (['tcp', 'quic'] if args.kind == 'both' else [args.kind]):
            print('Starting '+kind+' loopback '+args.profile, flush=True)
            result['runs'].append(run(kind, args.profile, root))
        result['completed'] = True
    finally:
        (root/'summary.json').write_text(json.dumps(result, indent=2)+'\n', encoding='utf-8', newline='\n')
        print('Text reports retained at .local/'+root.name, flush=True)


if __name__ == '__main__':
    main()
