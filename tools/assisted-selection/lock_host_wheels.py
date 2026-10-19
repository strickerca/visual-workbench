#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Inventory exact offline converter wheels and their bundled notices/native bytes."""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path
import re
import sys
from zipfile import ZipFile

import verify_artifacts as verify


def digest(stream):
    state = hashlib.sha256()
    count = 0
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        count += len(block)
        state.update(block)
    return count, state.hexdigest()


def key(value):
    return re.sub(r"[-_.]+", "-", value).lower()


def filename_is_notice(path):
    name = path.rsplit("/", 1)[-1].lower()
    return bool(name) and ("/licenses/" in path.lower() or
                           name.startswith(("license", "licence", "copying", "notice", "third_party_licenses")))


def main(wheelhouse, output, requirements, notices):
    catalog = verify.load_catalog()
    installed = {key(dist.metadata["Name"]): dist.version for dist in importlib.metadata.distributions()}
    wheels = {}
    notices.mkdir(mode=0o700, parents=True, exist_ok=True)
    for path in sorted(wheelhouse.glob("*.whl")):
        verify.require(path.is_file() and not path.is_symlink(), "wheel path")
        with ZipFile(path) as archive:
            entries = archive.infolist()
            verify.require(len(entries) <= 20000 and path.stat().st_size <= 256 * 1024 * 1024, "wheel bounds")
            metadata = [item for item in entries if len(Path(item.filename).parts) == 2 and
                        item.filename.endswith(".dist-info/METADATA")]
            verify.require(len(metadata) == 1 and metadata[0].file_size <= 4 * 1024 * 1024, "wheel metadata")
            text = archive.read(metadata[0]).decode("utf-8")
            from email.parser import Parser
            fields = Parser().parsestr(text)
            name, version = fields["Name"], fields["Version"]
            if installed.get(key(name)) != version:
                continue  # Resolver fetched another candidate, never installed.
            verify.require(key(name) not in wheels, "duplicate installed wheel")
            license_value = fields.get("License-Expression") or fields.get("License") or ""
            if len(license_value) > 160:
                license_value = license_value.splitlines()[0][:160]
            with path.open("rb") as wheel_stream:
                wheel_size, wheel_hash = digest(wheel_stream)
            row = {"distribution": name, "version": version, "wheel": path.name,
                   "bytes": wheel_size, "sha256": wheel_hash,
                   "metadata_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                   "license_metadata": license_value,
                   "license_classifiers": fields.get_all("Classifier", []),
                   "requires_dist": fields.get_all("Requires-Dist", []),
                   "notices": [], "native_files": []}
            for item in entries:
                if item.is_dir():
                    continue
                base = item.filename.rsplit("/", 1)[-1]
                native = ".so" in base or base.endswith((".dll", ".dylib", ".pyd"))
                notice = filename_is_notice(item.filename)
                if not (native or notice):
                    continue
                verify.require(item.file_size <= 512 * 1024 * 1024 and ".." not in Path(item.filename).parts,
                               "native/notice entry bounds")
                with archive.open(item) as stream:
                    size, sha = digest(stream)
                record = {"path": item.filename, "bytes": size, "sha256": sha}
                if native:
                    row["native_files"].append(record)
                if notice:
                    verify.require(size <= 4 * 1024 * 1024, "notice bounds")
                    target = notices / (key(name) + "-" + version) / ("notice-" + str(len(row["notices"])).zfill(3))
                    target.parent.mkdir(mode=0o700, exist_ok=True)
                    if target.exists():
                        verify.require(target.is_file() and not target.is_symlink(), "existing notice path")
                        with target.open("rb") as existing:
                            verify.require(digest(existing) == (size, sha), "existing notice changed")
                    else:
                        with archive.open(item) as source, target.open("xb") as destination:
                            for block in iter(lambda: source.read(1024 * 1024), b""):
                                destination.write(block)
                    record["copy"] = str(target.relative_to(notices))
                    row["notices"].append(record)
            wheels[key(name)] = row
    missing = sorted(name for name in installed if name not in wheels)
    verify.require(not missing, "installed wheels missing from wheelhouse: " + ", ".join(missing))
    for name, version in catalog["converter"]["packages"].items():
        verify.require(wheels[key(name)]["version"] == version, "primary package pin: " + name)
    rows = [wheels[name] for name in sorted(wheels)]
    python_binary = Path(sys.executable).resolve()
    with python_binary.open("rb") as source:
        python_sha = digest(source)[1]
    build_file = python_binary.parent.parent / "BUILD"
    verify.require(build_file.is_file(), "managed Python build identity")
    lock = {"schema": "vw-assisted-host-wheel-lock-v1", "python": sys.version.split()[0],
            "implementation": sys.implementation.name, "platform": sys.platform,
            "python_binary_sha256": python_sha, "python_build": build_file.read_text().strip(),
            "model_source_commit": catalog["model"]["source_commit"],
            "resource_policy": {"workers": 2, "address_space_bytes": 10 * 1024**3,
                                "output_file_bytes": 512 * 1024**2, "wall_seconds": 1200,
                                "network": "seccomp deny socket/connect/send/recv/io_uring"},
            "wheels": rows,
            "notice_review": "Bundled host notices and native inventories retained outside Git; human license review remains required before production admission."}
    output.write_text(json.dumps(lock, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n", encoding="ascii")
    requirements.write_text("".join(f"{row['distribution']}=={row['version']} --hash=sha256:{row['sha256']}\n"
                                    for row in rows), encoding="ascii")
    print("HOST_LOCK_OK packages=" + str(len(rows)) + " notices=" +
          str(sum(len(row["notices"]) for row in rows)) + " native_files=" +
          str(sum(len(row["native_files"]) for row in rows)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheelhouse", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--requirements", type=Path, required=True)
    parser.add_argument("--notices", type=Path, required=True)
    args = parser.parse_args()
    main(args.wheelhouse, args.output, args.requirements, args.notices)
