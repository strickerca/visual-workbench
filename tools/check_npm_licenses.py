"""Offline, exact-lock license admission for the bundled MCP server.

The reviewed policy supplements npm's integrity-checked, scripts-disabled install.
It is not inferred afresh from mutable package metadata at build time.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat

from check_setup import ALLOWED

ROOT = Path(__file__).resolve().parents[1]


def require(ok: bool, code: str) -> None:
    if not ok:
        raise ValueError(code)


def checked(root: Path, name: str, limit: int = 4 * 1024 * 1024) -> bytes:
    require(isinstance(name, str) and not name.startswith("/") and "\\" not in name
            and ":" not in name and all(p not in ("", ".", "..") for p in name.split("/")), "path")
    path = root / name
    require(path.resolve().is_relative_to(root.resolve()), "containment")
    for item in (path, *path.parents):
        info = item.lstat()
        require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                "redirect")
    before = path.stat()
    require(path.is_file() and before.st_size <= limit, "file_bound")
    with path.open("rb") as source:
        opened = os.fstat(source.fileno())
        require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
                (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns), "file_changed")
        data = source.read(limit + 1)
        after = os.fstat(source.fileno())
    final = path.lstat()
    require(len(data) <= limit and len(data) == before.st_size, "file_bound")
    require((after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns) ==
            (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
            (final.st_dev, final.st_ino, final.st_size, final.st_mtime_ns), "file_changed")
    return data


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def verify(root: Path, *, installed: Path | None = None, production: bool = False,
           node: Path | None = None) -> dict:
    policy = json.loads(checked(root, "tools/licenses/reviewed-npm.json"))
    require(policy.get("schema") == 1, "policy_schema")
    lock_bytes = checked(root, "mcp/package-lock.json")
    package_bytes = checked(root, "mcp/package.json")
    require(sha(lock_bytes) == policy["lock_sha256"] and sha(package_bytes) == policy["package_sha256"],
            "unreviewed_lock")
    lock = json.loads(lock_bytes)
    require(lock.get("lockfileVersion") == 3, "lock_version")
    rows = policy["packages"]
    require(len(rows) == len({r["path"] for r in rows}) and len(rows) <= 512, "policy_entries")
    require(set(lock["packages"]) == {""} | {r["path"] for r in rows}, "graph")
    admitted = []
    for row in rows:
        path = row["path"]
        require(re.fullmatch(r"node_modules/(?:@[a-z0-9._-]+/)?[a-z0-9._-]+", path) is not None,
                "module_path")
        entry = lock["packages"][path]
        require(row["license"] in ALLOWED, "license")
        require(all(entry.get(k) == row[k] for k in ("version", "integrity", "resolved", "license"))
                and bool(entry.get("dev")) == row["dev"], "module_binding")
        require(re.fullmatch(r"sha512-[A-Za-z0-9+/]{86}==", row["integrity"]) is not None
                and row["resolved"].startswith("https://registry.npmjs.org/"), "artifact_origin")
        for notice in row["notices"]:
            require(sha(checked(root, "third_party/" + notice["retained"])) == notice["sha256"],
                    "notice_binding")
        if production and row["dev"]:
            continue
        if installed is not None:
            meta_bytes = checked(installed, path + "/package.json")
            require(sha(meta_bytes) == row["package_sha256"], "installed_metadata")
            meta = json.loads(meta_bytes)
            require(meta.get("version") == row["version"] and meta.get("license") == row["license"],
                    "installed_identity")
            for notice in row["notices"]:
                require(sha(checked(installed, path + "/" + notice["name"])) == notice["sha256"],
                        "installed_notice")
        admitted.append(path)
    if installed is not None:
        # This reviewed graph has no nested dependencies or executable shims.
        # Refuse extras rather than silently shipping an unreviewed package.
        modules = installed / "node_modules"
        found = set()
        for item in modules.iterdir():
            info = item.lstat()
            require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                    "redirect")
            if item.name == ".package-lock.json" and item.is_file():
                continue
            if item.name == ".bin" and item.is_dir() and not production:
                continue
            require(item.is_dir() and not item.name.startswith("."), "extra_module")
            children = list(item.iterdir()) if item.name.startswith("@") else [item]
            for child in children:
                found.add(child.relative_to(installed).as_posix())
                require(not (child / "node_modules").exists(), "nested_module")
        require(found == set(admitted), "installed_graph")
    if node is not None:
        runtime = json.loads(checked(root, "tools/licenses/node-runtime.json"))
        require(runtime["signature_verified"] is True, "runtime_signature")
        require(sha(checked(node, "node.exe", 256 * 1024 * 1024)) == runtime["node_sha256"], "node_binary")
        require(sha(checked(node, "LICENSE")) == runtime["license_sha256"], "node_notice")
        tool_files = runtime["npm_inventory"]
        require(1 <= len(tool_files) <= 20000 and "node_modules/npm/bin/npm-cli.js" in tool_files,
                "npm_tool_inventory")
        for name, record in tool_files.items():
            require(name.startswith("node_modules/npm/"), "npm_tool_path")
            data = checked(node, name, 16 * 1024 * 1024)
            require(len(data) == record["bytes"] and sha(data) == record["sha256"], "npm_tool_bytes")
        current = set()
        for path in (node / "node_modules/npm").rglob("*"):
            info = path.lstat()
            require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                    "npm_tool_redirect")
            if path.is_file():
                current.add(path.relative_to(node).as_posix())
            require(len(current) <= 20000, "npm_tool_inventory")
        require(current == set(tool_files), "npm_tool_extra")
    return {"schema": 1, "lock_sha256": sha(lock_bytes), "reviewed_packages": len(rows),
            "admitted_packages": len(admitted), "installed_verified": installed is not None,
            "production": production, "node_verified": node is not None}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--installed", type=Path)
    parser.add_argument("--production", action="store_true")
    parser.add_argument("--node", type=Path)
    args = parser.parse_args()
    print("NPM LICENSES: PASS " + json.dumps(verify(ROOT, installed=args.installed,
          production=args.production, node=args.node), sort_keys=True))


if __name__ == "__main__":
    main()
