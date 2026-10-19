#!/usr/bin/env python3
"""Compile the exact bounded positive drawing evaluator; no GUI/input/capture.

The actual joined native producer and app/ABI integration remain separate work.
"""
from pathlib import Path
import subprocess
import tempfile

root=Path(__file__).resolve().parents[2]
deps=root/'target/debug/deps'
source=root/'host-win/crates/vw-remote-host/src/editor_probe_runner/compatibility/evidence.rs'
def latest(name):
    return max(deps.glob('lib'+name+'-*.rlib'),key=lambda p:p.stat().st_mtime)
with tempfile.TemporaryDirectory(prefix='vw-paint-drawing-evidence-') as directory:
    temporary=Path(directory)
    harness=temporary/'harness.rs'
    harness.write_text('pub use vw_remote::{Error,Result};\n#[path='+repr(str(source)).replace("'",'"')+'] mod evidence;\n')
    output=temporary/'evidence'
    command=['rustc','--edition=2024','--test','-D','warnings','-L','dependency='+str(deps)]
    for name in ('vw_remote','serde_json','ring'):
        command+=['--extern',name+'='+str(latest(name))]
    subprocess.run(command+[str(harness),'-o',str(output)],check=True,timeout=45)
    subprocess.run([str(output),'--nocapture'],check=True,timeout=20)
print('Exact drawing evaluator passed; no native ownership, GUI/input/capture or ABI execution claimed.')
