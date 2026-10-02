"""Live negative check: a certificate from a different owned server must be refused."""
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]
EXE = ROOT/'target/release/transport-bench.exe'
subprocess.run([sys.executable, str(ROOT/'tools/bench/transport/build_receipt.py'), 'check'],
               cwd=ROOT, timeout=30, check=True)
root = ROOT/'.local'/('transport-tls-negative-'+uuid.uuid4().hex)
root.mkdir()
children, logs = [], []
try:
    for name in ['target', 'different-identity']:
        folder = root/name
        log = (root/(name+'.log')).open('wb')
        logs.append(log)
        child = subprocess.Popen([str(EXE), 'serve', 'quic', '127.0.0.1:0', str(folder), '30'],
                                 stdout=log, stderr=subprocess.STDOUT, cwd=ROOT)
        children.append(child)
        deadline = time.monotonic()+10
        while not (folder/'ready.json').exists():
            if child.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('Owned TLS fixture server failed readiness')
            time.sleep(.1)
    port = json.loads((root/'target/ready.json').read_text())['port']
    output = root/'must-not-complete.json'
    result = subprocess.run([str(EXE), 'run', 'quic', f'127.0.0.1:{port}',
                             str(root/'different-identity/server.der'), str(output), 'smoke'],
                            cwd=ROOT, timeout=25, capture_output=True)
    if result.returncode == 0 or output.stat().st_size != 0:
        raise RuntimeError('Incorrect certificate was accepted or produced completion evidence')
    (root/'result.json').write_text(json.dumps({'different_certificate_rejected': True,
        'client_exit_code': result.returncode, 'completed_report_written': False}, indent=2)+'\n')
    print('PASS: different server certificate refused before measurement; no completed report.')
finally:
    for child in children:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
    for log in logs:
        log.close()
    print('Owned TLS fixture processes closed; receipts .local/'+root.name)
