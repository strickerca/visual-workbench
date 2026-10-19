#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Network preparation of three exact PyPI source-license archives; no model IO."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import stat
import urllib.request


SOURCES = (
    ("ordered-set-4.1.0.tar.gz", 12826,
     "694a8e44c87657c59292ede72891eb91d34131f6531463aab3009191c77364a8",
     "https://files.pythonhosted.org/packages/4c/ca/bfac8bc689799bcca4157e0e0ced07e70ce125193fc2e166d2e685b7e2fe/ordered-set-4.1.0.tar.gz"),
    ("sentencepiece-0.2.2.tar.gz", 8218435,
     "3d2b5e824b5622038dc7b490897efe05ebbbb9e7350fc142f3ecc8789ef9bdf6",
     "https://files.pythonhosted.org/packages/cc/33/ea3cb3839607eb175da835244a798f797f478c5ddf0e8ecdf57ea85a4c70/sentencepiece-0.2.2.tar.gz"),
    ("tokenizers-0.23.3.tar.gz", 385286,
     "cded33237c77caeef62944d32aa9a7ef42bdce2b3497e18d137e072a8c4be438",
     "https://files.pythonhosted.org/packages/e0/7c/2cabb2174e772636683008f2c5621949b645da7d303c596589e84516a184/tokenizers-0.23.3.tar.gz"),
)


def identity(path: Path, size: int, sha: str) -> None:
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_size != size:
        raise ValueError("sdist size/type: " + path.name)
    state = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            state.update(block)
    if state.hexdigest() != sha:
        raise ValueError("sdist hash: " + path.name)


def prepare(root: Path) -> None:
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    for name, size, sha, url in SOURCES:
        destination = root / name
        if destination.exists() or destination.is_symlink():
            identity(destination, size, sha)
            print("REUSED " + name, flush=True)
            continue
        temporary = root / (name + ".partial-" + str(os.getpid()))
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        state = hashlib.sha256()
        count = 0
        try:
            with os.fdopen(descriptor, "wb") as output, urllib.request.urlopen(url, timeout=30) as source:
                while True:
                    block = source.read(1024 * 1024)
                    if not block:
                        break
                    count += len(block)
                    if count > size:
                        raise ValueError("sdist exceeded byte pin: " + name)
                    state.update(block)
                    output.write(block)
                output.flush()
                os.fsync(output.fileno())
            if count != size or state.hexdigest() != sha or destination.exists():
                raise ValueError("sdist identity or destination changed: " + name)
            temporary.rename(destination)
            print("VERIFIED " + name + " bytes=" + str(size) + " sha256=" + sha, flush=True)
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--private-root", required=True, type=Path)
    args = parser.parse_args()
    prepare(args.private_root / "license-audit-inputs")
