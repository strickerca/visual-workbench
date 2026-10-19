#!/usr/bin/env python3
"""Compile exact Paint JSON projection/receipt functions with cached Rust deps.

This does not compile Windows helper ownership, UniFFI, JNI, Compose or the app.
Known producer types are the actual cached vw-remote crate, never fake targets.
"""
from pathlib import Path
import json
import subprocess
import tempfile

root = Path(__file__).resolve().parents[2]
deps = root / "target/debug/deps"
def latest(pattern):
    return max(deps.glob(pattern), key=lambda p:p.stat().st_mtime)
remote = latest("libvw_remote-*.rlib")
fingerprints = root / "target/debug/.fingerprint"
fingerprint = fingerprints / ("vw-remote-" + remote.stem.split("-")[1])
requirements = {name:value for _,name,_,value in json.loads((fingerprint/"lib-vw_remote.json").read_text())["deps"]}
def matched(name):
    for p in fingerprints.glob(name+"-*/lib-"+name):
        if int.from_bytes(bytes.fromhex(p.read_text()),"little")==requirements[name]:
            return deps / ("lib"+name+"-"+p.parent.name.split("-")[-1]+".rlib")
    raise SystemExit("Matching cached artifact unavailable: "+name)
projection = root / "host-win/crates/vw-remote-host/src/editor_probe_runner/paint_observation.rs"
native = root / "core/crates/vw-ffi/src/session/remote_compatibility.rs"
p = projection.read_text()
n = native.read_text()
body = p[p.index("#[derive(Clone, Debug, PartialEq, Eq)]"):p.index("impl PaintObservationHelper")]
body += p[p.index("#[cfg(test)]"):]
receipt = n[n.index("fn receipt("):n.index("#[uniffi::export]")]
native_tests = n[n.index("#[cfg(test)]"):].replace("mod tests", "mod receipt_tests", 1)
source = """use serde::Deserialize;
use vw_remote::{Target,profile::ImageIdentity,Error,Result};
const MAX_PROBE_OUTPUT:usize=1024*1024;
#[derive(Debug)] enum SessionError {Invalid,Backpressure}
type SessionResult<T>=std::result::Result<T,SessionError>;
""" + body + receipt + native_tests
with tempfile.TemporaryDirectory(prefix="vw-paint-projection-") as temporary:
    temporary = Path(temporary)
    path = temporary / "projection.rs"
    path.write_text(source)
    output = temporary / "projection"
    command = ["rustc", "--edition=2024", "--test", "-D", "warnings", "-L", "dependency="+str(deps)]
    for name,artifact in (("serde",matched("serde")),("serde_json",matched("serde_json")),("vw_remote",remote)):
        command += ["--extern", name+"="+str(artifact)]
    subprocess.run(command+[str(path),"-o",str(output)],cwd=root,check=True,timeout=45)
    subprocess.run([str(output),"--nocapture"],check=True,timeout=15)
print("Exact projection/receipt tests passed; temporary compiler outputs disposed.")
