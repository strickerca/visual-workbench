"""Build-owner-only, offline exact MCP inventories; never launches a process.

Server resources are generated BEFORE the application JAR. The separate installed
bridge is compiled AFTER jpackage with the completed image manifest embedded.
Neither inventory hashes its own containing executable/JAR.
"""
from __future__ import annotations
import argparse
import hashlib
import os
from pathlib import Path
import re
import stat

MAX_FILES = 20_000
MAX_BYTES = 1024 * 1024 * 1024
MAX_FILE = 256 * 1024 * 1024
MAX_MANIFEST = 4 * 1024 * 1024
SERVER_REQUIRED = {"vw-mcp-host.exe", "vw-mcp-package.exe", "node.exe", "mcp/src/main.mjs", "vw-codex-host.exe", "mcp/codex/main.mjs", "mcp/codex/client.mjs", "mcp/codex/package.mjs", "mcp/codex/process.mjs", "mcp/codex/schema.mjs", "mcp/codex/schema-worker.mjs", "mcp/codex/image-profiles.json"}
RESERVED = {"CON", "PRN", "AUX", "NUL"} | {f"{x}{n}" for x in ("COM", "LPT") for n in range(1, 10)}

def path_name(name: str) -> str:
    parts = name.split("/")
    if not 1 <= len(name) <= 256 or len(parts) > 32 or any(
        not re.fullmatch(r"[A-Za-z0-9._@+ -]+", p) or p in (".", "..") or p.endswith((".", " "))
        or p.split(".")[0].upper() in RESERVED for p in parts
    ):
        raise ValueError("inventory_path")
    return name

def plain(path: Path, directory: bool | None = None) -> os.stat_result:
    value = path.lstat()
    if stat.S_ISLNK(value.st_mode) or getattr(value, "st_file_attributes", 0) & 0x400:
        raise ValueError("redirect")
    if directory is True and not stat.S_ISDIR(value.st_mode) or directory is False and not stat.S_ISREG(value.st_mode):
        raise ValueError("file_type")
    return value

def scan(root: Path, *, exclude: set[str] = frozenset()) -> list[tuple[str, int, str]]:
    root = root.absolute()
    if root.resolve() != root:
        raise ValueError("canonical_root")
    for parent in reversed((root, *root.parents)):
        plain(parent, True)
    rows, seen, total, nodes = [], set(), 0, 0
    stack = [root]
    while stack:
        folder = stack.pop()
        plain(folder, True)
        with os.scandir(folder) as children:
            for child in children:
                nodes += 1
                if nodes > 30_000:
                    raise ValueError("inventory_nodes")
                path = Path(child.path)
                name = path_name(path.relative_to(root).as_posix())
                value = plain(path)
                if stat.S_ISDIR(value.st_mode):
                    if len(path.relative_to(root).parts) >= 32:
                        raise ValueError("inventory_depth")
                    stack.append(path)
                    continue
                plain(path, False)
                if name in exclude:
                    continue
                if name.lower() in seen or len(rows) >= MAX_FILES or not 0 <= value.st_size <= MAX_FILE:
                    raise ValueError("inventory_bound")
                seen.add(name.lower())
                total += value.st_size
                if total > MAX_BYTES:
                    raise ValueError("inventory_bytes")
                digest, count = hashlib.sha256(), 0
                with path.open("rb") as stream:
                    opened = os.fstat(stream.fileno())
                    if (opened.st_dev, opened.st_ino, opened.st_size) != (value.st_dev, value.st_ino, value.st_size):
                        raise ValueError("changed")
                    while block := stream.read(65536):
                        count += len(block)
                        if count > value.st_size:
                            raise ValueError("changed")
                        digest.update(block)
                    after = os.fstat(stream.fileno())
                    if (after.st_size, after.st_mtime_ns, count) != (value.st_size, value.st_mtime_ns, value.st_size):
                        raise ValueError("changed")
                final = plain(path, False)
                if (final.st_dev, final.st_ino, final.st_size, final.st_mtime_ns) != (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns):
                    raise ValueError("changed")
                rows.append((digest.hexdigest(), count, name))
    return sorted(rows, key=lambda row: row[2])

def encoded(rows: list[tuple[str, int, str]]) -> bytes:
    data = "".join(f"{hash_} {size} {name}\n" for hash_, size, name in rows).encode("ascii")
    if not 1 <= len(data) <= MAX_MANIFEST:
        raise ValueError("manifest_bound")
    return data

def write_new(path: Path, data: bytes) -> None:
    plain(path.parent, True)
    with path.open("xb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())

def server(stage: Path, resources: Path) -> bytes:
    rows = scan(stage)
    names = {row[2] for row in rows}
    if not SERVER_REQUIRED <= names or any(name in ("vw-mcp.exe", "VisualWorkbench.exe", "VisualWorkbenchDev.exe") or name.endswith(".jar") for name in names):
        raise ValueError("server_inventory")
    # Runtime names are a stricter subset than jpackage names.
    if any(" " in name for name in names):
        raise ValueError("server_path")
    manifest = encoded(rows)
    resources.mkdir()  # Exclusive ownership; never mutates a prior output.
    (resources / "mcp-server").mkdir()
    for expected, size, name in rows:
        target = resources / "mcp-server" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        digest, count = hashlib.sha256(), 0
        with (stage / name).open("rb") as source, target.open("xb") as output:
            while block := source.read(65536):
                count += len(block)
                if count > size:
                    raise ValueError("stage_changed")
                output.write(block)
                digest.update(block)
            output.flush()
            os.fsync(output.fileno())
        if count != size or digest.hexdigest() != expected:
            raise ValueError("stage_changed")
    write_new(resources / "vw-mcp-server.sha256", manifest)
    return manifest

def app_image(image: Path, output: Path, launcher: str) -> bytes:
    if launcher not in ("VisualWorkbench.exe", "VisualWorkbenchDev.exe"):
        raise ValueError("launcher")
    if (image / "vw-mcp.exe").exists() or (image / "vw-app-image.sha256").exists():
        raise ValueError("app_already_finalized")
    rows = scan(image)
    names = {row[2] for row in rows}
    if names.intersection({"VisualWorkbench.exe", "VisualWorkbenchDev.exe"}) != {launcher} or not any(name.startswith("app/") and name.endswith(".jar") for _, _, name in rows):
        raise ValueError("app_layout")
    if not any(name.startswith("runtime/") for _, _, name in rows):
        raise ValueError("runtime_missing")
    manifest = encoded(rows)
    write_new(output, manifest)
    write_new(image / "vw-app-image.sha256", manifest)
    return manifest

def main() -> None:
    parser = argparse.ArgumentParser()
    modes = parser.add_subparsers(dest="mode", required=True)
    p = modes.add_parser("server"); p.add_argument("stage", type=Path); p.add_argument("resources", type=Path)
    p = modes.add_parser("app-image"); p.add_argument("image", type=Path); p.add_argument("output", type=Path); p.add_argument("--launcher", choices=("VisualWorkbench.exe", "VisualWorkbenchDev.exe"), required=True)
    args = parser.parse_args()
    data = server(args.stage, args.resources) if args.mode == "server" else app_image(args.image, args.output, args.launcher)
    print(f"MCP_INVENTORY bytes={len(data)} sha256={hashlib.sha256(data).hexdigest()}")

if __name__ == "__main__":
    main()
