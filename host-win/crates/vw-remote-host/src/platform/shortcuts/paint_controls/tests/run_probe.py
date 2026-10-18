#!/usr/bin/env python3
"""Run exact pure Paint predicates, without Cargo, Windows or provider IO.

The actual module is compiled unchanged as a child module. No mock Windows
bindings, replacement implementations or installed-editor claims are used.
"""
from pathlib import Path
import subprocess
import tempfile


def main() -> None:
    module = Path(__file__).resolve().parents[1] / "compatibility.rs"
    with tempfile.TemporaryDirectory(prefix="vw-paint-facts-") as temp:
        source = Path(temp) / "probe.rs"
        source.write_text(f'#[path = "{module.as_posix()}"]\nmod compatibility;\n')
        executable = Path(temp) / "probe"
        subprocess.run(["rustc", "--edition=2024", "-Dwarnings", "--test", str(source),
                        "-o", str(executable)], check=True, timeout=20)
        subprocess.run([str(executable), "--test-threads=1"], check=True, timeout=10)
    print("Exact pure Paint fact tests passed; temporary Rust executable disposed.")


if __name__ == "__main__":
    main()
