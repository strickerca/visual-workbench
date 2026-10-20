#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Resolve preserved notice proposals without admitting them to third_party."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import stat


REPOSITORY = Path(__file__).resolve().parents[3]
MANIFEST = "docs/license-proposals/disposition.json"
PREFIXES = {
    "third_party/notices/assisted-selection/": "docs/license-proposals/notices/assisted-selection/",
    "third_party/notices/scrcpy/": "docs/license-proposals/notices/scrcpy/",
}
COUNTS = {"assisted-selection": 118, "scrcpy": 7, "total": 125}
RETAINED_RECORDS = frozenset({
    "tools/assisted-selection/license-audit/notice-sources.json",
    "tools/assisted-selection/license-audit/component-census.json",
    "tools/assisted-selection/license-audit/root-component-proposals.json",
    "tools/android-tunnel/pinned-runtime-inventory.json",
    "tools/android-tunnel/root-manifest-proposal.json",
})
MAX_NOTICE = 4 * 1024 * 1024


class DispositionError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise DispositionError(message)


def destination(source: str) -> str:
    """Translate only an explicit former proposal path, never a package path."""
    require(isinstance(source, str) and bool(source) and "\\" not in source and
            not PurePosixPath(source).is_absolute() and ".." not in PurePosixPath(source).parts and
            PurePosixPath(source).as_posix() == source, "unsafe notice proposal path")
    for old, new in PREFIXES.items():
        if source.startswith(old) and len(source) > len(old):
            return new + source[len(old):]
    raise DispositionError("unmapped notice proposal path: " + source)


def file_identity(path: Path, maximum: int = MAX_NOTICE) -> tuple[int, str]:
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= maximum,
            "regular bounded proposal file required: " + str(path))
    with path.open("rb") as source:
        content = source.read(maximum + 1)
    require(len(content) == info.st_size, "proposal file changed while reading: " + str(path))
    return len(content), hashlib.sha256(content).hexdigest()


def _identity_record(row: dict) -> None:
    require(type(row.get("bytes")) is int and 0 < row["bytes"] <= MAX_NOTICE and
            isinstance(row.get("sha256"), str) and
            re.fullmatch(r"[0-9a-f]{64}", row["sha256"]) is not None,
            "invalid proposal identity record")


def load(repo: Path = REPOSITORY) -> dict:
    path = repo / MANIFEST
    file_identity(path, 128 * 1024)
    with path.open("rb") as source:
        raw = source.read(128 * 1024 + 1)
    require(len(raw) <= 128 * 1024, "notice disposition byte limit")
    manifest = json.loads(raw)
    require(type(manifest) is dict and manifest.get("schema") == "vw-unadmitted-notice-disposition-v1" and
            manifest.get("status") == "unadmitted-proposals-outside-packaged-tree" and
            manifest.get("production_admitted") is False and manifest.get("counts") == COUNTS,
            "notice disposition header must remain unadmitted")
    rows = manifest.get("files")
    require(type(rows) is list and len(rows) == COUNTS["total"], "notice disposition count")
    seen = set()
    counts = {group: 0 for group in COUNTS if group != "total"}
    for row in rows:
        require(type(row) is dict and set(row) == {"source", "destination", "bytes", "sha256"},
                "notice disposition row")
        _identity_record(row)
        source = row["source"]
        require(row["destination"] == destination(source) and source not in seen,
                "notice disposition mapping differs or duplicates")
        seen.add(source)
        counts[source.split("/")[2]] += 1
    require({**counts, "total": len(rows)} == COUNTS and
            manifest.get("total_bytes") == sum(row["bytes"] for row in rows),
            "notice disposition totals")
    retained = manifest.get("retained_records")
    require(type(retained) is list and len(retained) == len(RETAINED_RECORDS), "retained record count")
    paths = set()
    for row in retained:
        require(type(row) is dict and set(row) == {"path", "bytes", "sha256"} and
                row["path"] in RETAINED_RECORDS and row["path"] not in paths,
                "retained record path")
        _identity_record(row)
        paths.add(row["path"])
    return manifest


def resolve(repo: Path, source: str, manifest: dict | None = None) -> Path:
    """Find a mapped proposal; never fall back to files in the packaged tree."""
    mapped = destination(source)
    manifest = load(repo) if manifest is None else manifest
    require(any(row["source"] == source and row["destination"] == mapped for row in manifest["files"]),
            "notice proposal is absent from disposition")
    path = repo.resolve() / mapped
    require(path.resolve() == path, "notice proposal path traverses a symlink")
    return path


def verify(repo: Path = REPOSITORY) -> dict:
    manifest = load(repo)
    expected = set()
    for row in manifest["files"]:
        path = resolve(repo, row["source"], manifest)
        require(file_identity(path) == (row["bytes"], row["sha256"]),
                "preserved notice identity differs: " + row["source"])
        expected.add(row["destination"])
    actual = {path.relative_to(repo).as_posix()
              for path in (repo / "docs/license-proposals/notices").rglob("*")
              if not path.is_dir() or path.is_symlink()}
    require(actual == expected, "proposal tree differs from disposition")
    for prefix in PREFIXES:
        require(not (repo / prefix).exists() and not (repo / prefix).is_symlink(),
                "unadmitted notice proposal restored inside packaged tree")
    for row in manifest["retained_records"]:
        require(file_identity(repo / row["path"]) == (row["bytes"], row["sha256"]),
                "historical record identity differs: " + row["path"])
    return manifest


if __name__ == "__main__":
    try:
        result = verify()
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise SystemExit("NOTICE_DISPOSITION_REFUSED " + str(error)) from error
    print("NOTICE_DISPOSITION_OK " + json.dumps({"counts": result["counts"],
          "total_bytes": result["total_bytes"]}, sort_keys=True) + " production_admitted=false")
