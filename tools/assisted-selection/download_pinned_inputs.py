#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Network-only preparation of exact public MobileSAM source/checkpoint bytes."""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import os
from pathlib import Path, PurePosixPath
import urllib.request

import verify_artifacts as verify


def fetch(url, destination, size, digest):
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    if destination.exists():
        verify.require(not destination.is_symlink(), "existing input link")
        with destination.open("rb") as source:
            data = source.read(size + 1)
        verify.require(len(data) == size and hashlib.sha256(data).hexdigest() == digest,
                       "existing input pin: " + destination.name)
        return "REUSED " + destination.name
    partial = destination.with_name(destination.name + ".partial-" + str(os.getpid()))
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    fd = os.open(partial, flags, 0o600)
    state = hashlib.sha256()
    count = 0
    try:
        with os.fdopen(fd, "wb") as output, urllib.request.urlopen(
            urllib.request.Request(url, headers={"User-Agent": "visual-workbench-pinned-prep/1"}), timeout=30
        ) as source:
            while True:
                block = source.read(1024 * 1024)
                if not block:
                    break
                count += len(block)
                verify.require(count <= size, "download exceeded pin")
                state.update(block)
                output.write(block)
            output.flush()
            os.fsync(output.fileno())
        verify.require(count == size and state.hexdigest() == digest, "download pin mismatch")
        verify.require(not destination.exists(), "destination appeared concurrently")
        partial.rename(destination)
        return "VERIFIED " + destination.name + " bytes=" + str(count) + " sha256=" + digest
    except BaseException:
        partial.unlink(missing_ok=True)
        raise


def main(root):
    catalog = verify.load_catalog()
    model = catalog["model"]
    commit = model["source_commit"]
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    source_root = root / "MobileSAM-pinned"
    source_root.mkdir(mode=0o700, exist_ok=True)
    jobs = []
    for row in model["source_files"]:
        parts = PurePosixPath(row["path"]).parts
        verify.require(bool(parts) and all(part not in ("", ".", "..") for part in parts), "source path")
        jobs.append(("https://raw.githubusercontent.com/ChaoningZhang/MobileSAM/" + commit + "/" + row["path"],
                     source_root.joinpath(*parts), row["bytes"], row["sha256"]))
    checkpoint = model["checkpoint"]
    verify.require(checkpoint["url"].startswith("https://raw.githubusercontent.com/ChaoningZhang/MobileSAM/" + commit + "/"),
                   "checkpoint source URL")
    jobs.append((checkpoint["url"], root / "mobile_sam.pt", checkpoint["bytes"], checkpoint["sha256"]))
    with ThreadPoolExecutor(max_workers=2) as workers:
        futures = [workers.submit(fetch, *job) for job in jobs]
        for future in as_completed(futures):
            print(future.result(), flush=True)
    print("INPUTS_OK source_root=" + str(source_root) + " checkpoint=" + str(root / "mobile_sam.pt"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    main(parser.parse_args().root)
