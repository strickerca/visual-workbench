#!/usr/bin/env python3
"""Prepare pinned scrcpy v4.1 Win64 bytes without executing them or shipping adb.

The official ZIP hash authenticates every member. The resulting inventory is
source evidence, not a license or runtime admission decision.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import tempfile
import time
from urllib.parse import urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SCHEMA = "vw.scrcpy-prepared.v1"
MAX_ARCHIVE = 16 * 1024 * 1024
MAX_INVENTORY = 16 * 1024
MAX_MEMBERS = 24
PREFIX = "scrcpy-win64-v4.1/"
RETAIN = frozenset({
    "scrcpy.exe", "scrcpy-server", "LICENSE.txt", "scrcpy.png", "disconnected.png",
    "SDL3.dll", "avcodec-62.dll", "avformat-62.dll", "avutil-60.dll",
    "swresample-6.dll", "libusb-1.0.dll",
})
OMIT = frozenset({"adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll",
                  "open_a_terminal_here.bat", "scrcpy-noconsole.vbs"})
PE_FILES = frozenset({"scrcpy.exe", "SDL3.dll", "avcodec-62.dll", "avformat-62.dll",
                      "avutil-60.dll", "swresample-6.dll", "libusb-1.0.dll"})
FFMPEG = frozenset({"avcodec-62.dll", "avformat-62.dll", "avutil-60.dll", "swresample-6.dll"})


class Refusal(ValueError):
    """Pinned bytes, ownership, or archive structure did not match."""


@dataclass(frozen=True)
class Pin:
    version: str
    url: str
    archive_bytes: int
    archive_sha256: str
    server_sha256: str
    members: dict[str, int]
    retain: frozenset[str]
    omit: frozenset[str]
    pe_files: frozenset[str]


# Official v4.1 doc/windows.md and doc/build.md publish the archive and server
# hashes. Archive/member sizes are a provisional census from Fossies; the SHA
# checks, not these independently sourced sizes, authenticate the release.
PIN = Pin(
    "4.1",
    "https://github.com/Genymobile/scrcpy/releases/download/v4.1/scrcpy-win64-v4.1.zip",
    11_305_298,
    "5b12172b3264b2889f4583ee64752ce832e29bc8b1089dca81093459697165db",
    "deacb991ed2509715160ffdc7907e47b4160eb30d1566217e9047fd5b8850cae",
    {
        "adb.exe": 8_485_016, "AdbWinApi.dll": 108_184, "AdbWinUsbApi.dll": 73_368,
        "avcodec-62.dll": 8_603_648, "avformat-62.dll": 723_456,
        "avutil-60.dll": 1_039_360, "swresample-6.dll": 139_776,
        "libusb-1.0.dll": 227_005, "SDL3.dll": 5_366_478,
        "scrcpy.exe": 720_524, "scrcpy-server": 733_706,
        "LICENSE.txt": 11_387, "scrcpy.png": 6_530, "disconnected.png": 4_703,
        "open_a_terminal_here.bat": 5, "scrcpy-noconsole.vbs": 212,
    }, RETAIN, OMIT, PE_FILES,
)


def _sha_file(path: Path, limit: int) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            size += len(chunk)
            if size > limit:
                raise Refusal("File exceeds its pinned size limit")
            digest.update(chunk)
    return size, digest.hexdigest()


def _bounded_json(path: Path) -> tuple[bytes, dict]:
    try:
        with path.open("rb") as source:
            raw = source.read(MAX_INVENTORY + 1)
        if len(raw) > MAX_INVENTORY:
            raise Refusal("Inventory exceeds its 16 KiB byte limit")
        parsed = json.loads(raw.decode("utf-8", errors="strict"))
        if not isinstance(parsed, dict):
            raise Refusal("Inventory must be a JSON object")
        return raw, parsed
    except (UnicodeError, json.JSONDecodeError, OSError) as error:
        raise Refusal("Inventory is missing or malformed") from error


def _canonical_inventory(inventory: dict) -> bytes:
    return (json.dumps(inventory, indent=2, sort_keys=True) + "\n").encode("utf-8")


def _outside_repository(path: Path, *, must_exist: bool) -> Path:
    if not path.is_absolute():
        raise Refusal("Use an absolute path outside the repository")
    if path.is_symlink() or (must_exist and not path.is_file()):
        raise Refusal("Expected a regular, non-symlink file")
    resolved = path.resolve(strict=must_exist)
    if resolved == ROOT or ROOT in resolved.parents:
        raise Refusal("Prepared and downloaded bytes must stay outside Git")
    return resolved


def _destination(path: Path) -> Path:
    if not path.is_absolute() or path.name in {"", ".", ".."}:
        raise Refusal("Destination must be an absolute new directory")
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir() or parent.resolve() != parent:
        raise Refusal("Destination parent must be a real existing directory")
    if ROOT == parent or ROOT in parent.parents:
        raise Refusal("Destination must be outside Git")
    if path.exists() or path.is_symlink():
        raise Refusal("Destination already exists")
    return path


def _pe_machine(path: Path) -> str:
    with path.open("rb") as source:
        header = source.read(64)
        if len(header) != 64 or header[:2] != b"MZ":
            raise Refusal(f"{path.name}: missing PE DOS header")
        offset = struct.unpack_from("<I", header, 0x3C)[0]
        if not 64 <= offset <= 1024 * 1024:
            raise Refusal(f"{path.name}: invalid PE offset")
        source.seek(offset)
        pe = source.read(26)
    if len(pe) != 26 or pe[:4] != b"PE\0\0" or struct.unpack_from("<H", pe, 4)[0] != 0x8664:
        raise Refusal(f"{path.name}: not an AMD64 PE image")
    if struct.unpack_from("<H", pe, 24)[0] != 0x20B:
        raise Refusal(f"{path.name}: not a PE32+ image")
    return "PE32+ AMD64 (0x8664)"


def _role(name: str) -> str:
    if name in FFMPEG:
        return "ffmpeg-dynamic"
    if name == "libusb-1.0.dll":
        return "libusb-dynamic"
    if name == "SDL3.dll":
        return "sdl-dynamic"
    if name == "scrcpy.exe":
        return "client"
    if name == "scrcpy-server":
        return "android-server"
    if name == "LICENSE.txt":
        return "scrcpy-license-text"
    return "client-resource"


def inspect_archive(archive: Path, pin: Pin = PIN, output: Path | None = None) -> dict:
    archive = _outside_repository(archive, must_exist=True)
    if not pin.retain.isdisjoint(pin.omit) or pin.retain | pin.omit != pin.members.keys():
        raise Refusal("Pin has an incomplete or overlapping member set")
    size, digest = _sha_file(archive, min(MAX_ARCHIVE, pin.archive_bytes))
    if size != pin.archive_bytes or digest != pin.archive_sha256:
        raise Refusal("Official archive size or SHA-256 mismatch")
    found: dict[str, zipfile.ZipInfo] = {}
    try:
        with zipfile.ZipFile(archive) as bundle:
            infos = bundle.infolist()
            if len(infos) > MAX_MEMBERS:
                raise Refusal("Too many ZIP entries")
            names_casefold: set[str] = set()
            root_seen = False
            for info in infos:
                name = info.filename
                if name == PREFIX and info.is_dir():
                    if root_seen:
                        raise Refusal("Duplicate ZIP root")
                    root_seen = True
                    if stat.S_IFMT(info.external_attr >> 16) not in (0, stat.S_IFDIR):
                        raise Refusal("Unsafe ZIP root type")
                    continue
                if not name.startswith(PREFIX) or name.count("/") != 1 or info.is_dir():
                    raise Refusal("Unknown or unsafe ZIP path")
                short = name[len(PREFIX):]
                if short not in pin.members or short.casefold() in names_casefold:
                    raise Refusal("Unknown or duplicate ZIP member")
                names_casefold.add(short.casefold())
                mode = stat.S_IFMT(info.external_attr >> 16)
                if mode not in (0, stat.S_IFREG) or info.flag_bits & 1 or info.file_size != pin.members[short]:
                    raise Refusal("ZIP member type, encryption or size mismatch")
                if info.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
                    raise Refusal("Unsupported ZIP compression")
                found[short] = info
            if not root_seen or found.keys() != pin.members.keys():
                raise Refusal("Incomplete official archive")
            records, excluded = [], []
            for name in sorted(found):
                info = found[name]
                sha = hashlib.sha256()
                total = 0
                target = output / name if output is not None and name in pin.retain else None
                sink = target.open("xb") if target is not None else None
                try:
                    with bundle.open(info) as member:
                        while chunk := member.read(1024 * 1024):
                            total += len(chunk)
                            if total > pin.members[name]:
                                raise Refusal("ZIP member exceeded pinned size")
                            sha.update(chunk)
                            if sink is not None:
                                sink.write(chunk)
                finally:
                    if sink is not None:
                        sink.close()
                if total != pin.members[name]:
                    raise Refusal("ZIP member truncated")
                record = {"name": name, "bytes": total, "sha256": sha.hexdigest()}
                if name in pin.retain:
                    if name == "scrcpy-server" and record["sha256"] != pin.server_sha256:
                        raise Refusal("Embedded server differs from official v4.1 server")
                    if name in pin.pe_files:
                        if target is None:
                            # The verification path still checks the PE bytes without executing them.
                            with tempfile.TemporaryDirectory(prefix="vw-scrcpy-pe-") as scratch:
                                candidate = Path(scratch) / name
                                with bundle.open(info) as member, candidate.open("xb") as sink_pe:
                                    shutil.copyfileobj(member, sink_pe, 1024 * 1024)
                                record["architecture"] = _pe_machine(candidate)
                        else:
                            record["architecture"] = _pe_machine(target)
                    else:
                        record["architecture"] = None
                    record["role"] = _role(name)
                    records.append(record)
                else:
                    record["disposition"] = "excluded; never extracted"
                    excluded.append(record)
    except (zipfile.BadZipFile, OSError) as error:
        raise Refusal(f"Archive could not be read safely: {error}") from error
    return {
        "schema": SCHEMA, "version": pin.version,
        "archive": {"url": pin.url, "bytes": size, "sha256": digest},
        "server_sha256": pin.server_sha256,
        "license_state": "pending-actual-DLL-review; not admitted for distribution",
        "files": records, "excluded": excluded,
    }


def _pinned_inventory() -> dict:
    raw, inventory = _bounded_json(Path(__file__).with_name("pinned-runtime-inventory.json"))
    if raw != _canonical_inventory(inventory):
        raise Refusal("Reviewed release inventory is not canonical")
    if inventory.get("schema") != SCHEMA or inventory.get("version") != PIN.version or \
       inventory.get("archive") != {"url": PIN.url, "bytes": PIN.archive_bytes, "sha256": PIN.archive_sha256} or \
       inventory.get("server_sha256") != PIN.server_sha256:
        raise Refusal("Reviewed release inventory does not match the immutable pin")
    files = inventory.get("files")
    excluded = inventory.get("excluded")
    if not isinstance(files, list) or not isinstance(excluded, list) or \
       {row.get("name") for row in files if isinstance(row, dict)} != PIN.retain or \
       {row.get("name") for row in excluded if isinstance(row, dict)} != PIN.omit or \
       len(files) != len(PIN.retain) or len(excluded) != len(PIN.omit):
        raise Refusal("Reviewed release inventory has an incomplete member census")
    for row in files + excluded:
        if not isinstance(row, dict) or row.get("bytes") != PIN.members[row.get("name")] or \
           not isinstance(row.get("sha256"), str) or len(row["sha256"]) != 64:
            raise Refusal("Reviewed release inventory has invalid file metadata")
    return inventory


def verify(prepared: Path, archive: Path | None = None, pin: Pin = PIN) -> dict:
    prepared = _outside_repository(prepared, must_exist=False)
    if not prepared.is_dir() or prepared.is_symlink():
        raise Refusal("Prepared runtime is not a plain directory")
    if archive is None and pin != PIN:
        raise Refusal("Synthetic pins require their source archive for verification")
    expected = inspect_archive(archive, pin) if archive is not None else _pinned_inventory()
    if pin == PIN and expected != _pinned_inventory():
        raise Refusal("Official archive differs from the reviewed per-file inventory")
    wanted = pin.retain | {"inventory.json"}
    actual = {p.name for p in prepared.iterdir()}
    if actual != wanted or any(not p.is_file() or p.is_symlink() for p in prepared.iterdir()):
        raise Refusal("Prepared runtime is incomplete or contains unexpected entries")
    raw, inventory = _bounded_json(prepared / "inventory.json")
    if raw != _canonical_inventory(expected) or inventory != expected:
        raise Refusal("Prepared inventory differs from the pinned archive")
    for record in expected["files"]:
        path = prepared / record["name"]
        if _sha_file(path, record["bytes"]) != (record["bytes"], record["sha256"]):
            raise Refusal(f"Prepared file changed: {record['name']}")
        if record["name"] in pin.pe_files and _pe_machine(path) != record["architecture"]:
            raise Refusal("Prepared PE architecture changed")
    return expected


def prepare(archive: Path, destination: Path, pin: Pin = PIN) -> dict:
    destination = _destination(destination)
    stage = Path(tempfile.mkdtemp(prefix=".vw-scrcpy-prepare-", dir=destination.parent))
    try:
        inventory = inspect_archive(archive, pin, stage)
        (stage / "inventory.json").write_bytes(_canonical_inventory(inventory))
        verify(stage, archive, pin)
        # CREATE_NEW owns the destination. Exclusive file copies never replace
        # an entry that another process may have placed there in the meantime.
        destination.mkdir(mode=0o700)
        for source in sorted(stage.iterdir()):
            with source.open("rb") as reader, (destination / source.name).open("xb") as writer:
                shutil.copyfileobj(reader, writer, 1024 * 1024)
        return verify(destination, archive, pin)
    finally:
        if stage.exists():
            shutil.rmtree(stage)


def _approved_release_url(url: str) -> None:
    parsed = urlparse(url)
    try:
        port = parsed.port
    except ValueError as error:
        raise Refusal("Release redirect has an invalid port") from error
    if parsed.scheme != "https" or parsed.hostname not in {"github.com", "release-assets.githubusercontent.com"} or \
       parsed.username is not None or parsed.password is not None or port not in (None, 443) or parsed.fragment:
        raise Refusal("Release redirect used HTTP or an unapproved host")


class _ReleaseRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        _approved_release_url(request.full_url)
        _approved_release_url(new_url)
        return super().redirect_request(request, response, code, message, headers, new_url)


def download(destination: Path, pin: Pin = PIN) -> dict:
    destination = _destination(destination)
    with tempfile.TemporaryDirectory(prefix=".vw-scrcpy-download-", dir=destination.parent) as scratch:
        archive = Path(scratch) / "scrcpy-win64-v4.1.zip"
        _approved_release_url(pin.url)
        request = Request(pin.url, headers={"User-Agent": "visual-workbench-scrcpy-audit/1"})
        with build_opener(_ReleaseRedirects()).open(request, timeout=30) as response, archive.open("xb") as sink:
            _approved_release_url(response.geturl())
            total = 0
            deadline = time.monotonic() + 180
            while True:
                if time.monotonic() > deadline:
                    raise Refusal("Official release download exceeded its time budget")
                chunk = response.read(1024 * 1024)
                if not chunk:
                    break
                total += len(chunk)
                if total > min(pin.archive_bytes, MAX_ARCHIVE):
                    raise Refusal("Download exceeds the pinned archive size")
                sink.write(chunk)
        return prepare(archive, destination, pin)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "verify", "download"):
        command = commands.add_parser(name)
        command.add_argument("--destination", type=Path, required=True)
        if name == "prepare":
            command.add_argument("--archive", type=Path, required=True)
        elif name == "verify":
            command.add_argument("--archive", type=Path)
    args = parser.parse_args()
    try:
        result = (download(args.destination) if args.command == "download" else
                  prepare(args.archive, args.destination) if args.command == "prepare" else
                  verify(args.destination, args.archive))
    except (Refusal, OSError) as error:
        parser.exit(2, f"scrcpy runtime refused: {error}\n")
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
