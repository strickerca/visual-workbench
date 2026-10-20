#!/usr/bin/env python3
"""Assemble explicitly listed, already built bytes; never build, sign or install."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys
from types import SimpleNamespace
import zipfile

CHUNK = 1024 * 1024
MAX_JSON = 8 * CHUNK
MAX_ARTIFACTS = 4096
MAX_SOURCES = 24000
MAX_FILE = 2 * 1024**3
MAX_TOTAL = 8 * 1024**3
MAX_SOURCE_FILE = MAX_FILE
MAX_SOURCE_TOTAL = 4 * 1024**3
MAX_OUTPUT_ENTRIES = 32768
HASH = re.compile(r"[0-9a-f]{64}\Z")
DESKTOP = "apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev/"
DOCUMENTS = {"toolchain_record": "release-docs/TOOLCHAIN.txt",
             "uninstall_recovery_guide": "release-docs/UNINSTALL-RECOVERY.txt"}


class Rejected(ValueError):
    """Static refusal code, without host paths or untrusted exception text."""


def require(condition: bool, code: str) -> None:
    if not condition:
        raise Rejected(code)


def validators():
    # Load repository-owned validators, never code named by an input manifest.
    base = Path(__file__).absolute().parents[2]
    with _validator_imports(base):
        reports = sys.modules["reports"]
        personal = _module("vw_release_personal", base / "tools/desktop-test/personal_package.py")
    apk = _module("vw_release_apk", base / "tools/ffi-test/check_apk.py")
    return reports, personal, apk


def _module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    require(spec is not None and spec.loader is not None, "validator_unavailable")
    value = importlib.util.module_from_spec(spec)
    sys.modules[name] = value
    spec.loader.exec_module(value)
    return value


class _validator_imports:
    def __init__(self, base: Path):
        self.base = base
        self.previous = None

    def __enter__(self):
        self.previous = sys.modules.get("reports")
        _module("reports", self.base / "tools/desktop-test/reports.py")

    def __exit__(self, *_):
        if self.previous is None:
            sys.modules.pop("reports", None)
        else:
            sys.modules["reports"] = self.previous


def name(value: str) -> str:
    # Keep the existing cross-platform path contract authoritative.
    try:
        REPORTS.relative_name(value)
    except REPORTS.Rejected as error:
        raise Rejected("unsafe_relative_path") from error
    return value


def absolute(value: Path) -> Path:
    value = Path(value)
    require(value.is_absolute() and ".." not in value.parts, "absolute_path_required")
    if os.name == "nt":
        require(re.fullmatch(r"[A-Za-z]:\\", value.anchor) is not None and
                ":" not in str(value)[2:], "ordinary_local_path_required")
    for part in value.parts[1:]:
        name(part)
    return value


def identity(info) -> tuple:
    return info.st_dev, info.st_ino, stat.S_IFMT(info.st_mode)


def snapshot(info) -> tuple:
    return identity(info) + (info.st_size, info.st_mtime_ns, info.st_ctime_ns, info.st_nlink)


def windows_metadata(attributes: int, size: int, links: int, volume: int, file_id: bytes,
                     written: int, changed: int, created: int):
    """One native clock/identity representation for both path and handle reads.

    CPython 3.14's path stat replaces ChangeTime with CreationTime for legacy
    st_ctime, while fstat retains ChangeTime. Do not compare those two meanings
    or discard the change clock. FILE_BASIC_INFO supplies the same ChangeTime
    for every native metadata query below (100 ns ticks since 1601).
    https://github.com/python/cpython/blob/3.14/Modules/posixmodule.c
    https://github.com/python/cpython/blob/3.14/Python/fileutils.c
    """
    require(all(type(value) is int for value in (attributes, size, links, volume, written, changed, created)) and
            size >= 0 and links > 0 and volume >= 0 and len(file_id) == 16 and any(file_id), "native_metadata")
    epoch = 116444736000000000
    return SimpleNamespace(st_mode=stat.S_IFDIR if attributes & 0x10 else stat.S_IFREG,
                           st_dev=volume, st_ino=int.from_bytes(file_id, "little"), st_size=size,
                           st_mtime_ns=(written - epoch) * 100, st_ctime_ns=(changed - epoch) * 100,
                           st_birthtime_ns=(created - epoch) * 100, st_nlink=links,
                           st_file_attributes=attributes)


def windows_handle_metadata(handle, api=None):
    import ctypes
    from ctypes import wintypes
    if api is None:
        api = ctypes.WinDLL("kernel32", use_last_error=True)
        api.GetFileInformationByHandleEx.argtypes = (wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD)
        api.GetFileInformationByHandleEx.restype = wintypes.BOOL

    class Basic(ctypes.Structure):
        _fields_ = [("created", ctypes.c_int64), ("accessed", ctypes.c_int64),
                    ("written", ctypes.c_int64), ("changed", ctypes.c_int64), ("attributes", ctypes.c_uint32)]

    class Standard(ctypes.Structure):
        _fields_ = [("allocated", ctypes.c_int64), ("size", ctypes.c_int64), ("links", ctypes.c_uint32),
                    ("delete_pending", ctypes.c_ubyte), ("directory", ctypes.c_ubyte)]

    class FileId(ctypes.Structure):
        _fields_ = [("volume", ctypes.c_uint64), ("file_id", ctypes.c_ubyte * 16)]

    basic, standard, file_id = Basic(), Standard(), FileId()
    for kind, value in ((0, basic), (1, standard), (18, file_id)):
        require(bool(api.GetFileInformationByHandleEx(handle, kind, ctypes.byref(value), ctypes.sizeof(value))),
                "native_metadata_unavailable")
    require(not standard.delete_pending and bool(standard.directory) == bool(basic.attributes & 0x10),
            "native_metadata_type")
    return windows_metadata(basic.attributes, standard.size, standard.links, file_id.volume,
                            bytes(file_id.file_id), basic.written, basic.changed, basic.created)


def fstat_metadata(fd: int):
    if os.name != "nt":
        return os.fstat(fd)
    import msvcrt
    return windows_handle_metadata(msvcrt.get_osfhandle(fd))


def windows_path_metadata(path: Path, directory: bool | None = None):
    # A short metadata query must coexist with our output writer. Retained read
    # leases still deny write/delete; this query denies neither and detects
    # redirects/aliases and changes using the same native fields as fstat.
    lease = WindowsLease(path, directory, metadata_only=True)
    try:
        return lease.metadata()
    finally:
        lease.close()


def ordinary(info, directory: bool) -> None:
    require(not stat.S_ISLNK(info.st_mode) and
            not getattr(info, "st_file_attributes", 0) & 0x400, "redirected_path")
    require(stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode), "path_type")
    require(type(info.st_ino) is int and info.st_ino > 0, "identity_unavailable")
    if not directory:
        require(info.st_nlink == 1, "hardlinked_input")


class WindowsLease:
    """No-follow directory handle; refuse reparse points and resolved aliases.

    The handle denies delete sharing, as the existing desktop lease does. Opened
    file handles deny write/delete sharing while bytes are read. This backend is
    source-only until the deferred real-Windows tests have run.
    """
    def __init__(self, path: Path, directory: bool | None, *, metadata_only: bool = False):
        import ctypes
        from ctypes import wintypes
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateFileW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                       wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE)
        self.api.CreateFileW.restype = wintypes.HANDLE
        self.api.CloseHandle.argtypes = (wintypes.HANDLE,)
        self.api.GetFileInformationByHandleEx.argtypes = (wintypes.HANDLE, ctypes.c_int,
                                                         wintypes.LPVOID, wintypes.DWORD)
        self.api.GetFileInformationByHandleEx.restype = wintypes.BOOL
        self.api.GetFinalPathNameByHandleW.argtypes = (wintypes.HANDLE, wintypes.LPWSTR,
                                                     wintypes.DWORD, wintypes.DWORD)
        self.handle = self.api.CreateFileW(str(path), 0x80 if directory or metadata_only else 0x80000000,
                                          7 if metadata_only else (3 if directory else 1), None, 3,
                                          0x00200000 | (0x02000000 if directory or metadata_only else 0x80), None)
        require(self.handle not in (None, wintypes.HANDLE(-1).value), "path_lease_unavailable")
        try:
            info = (wintypes.DWORD * 2)()
            require(bool(self.api.GetFileInformationByHandleEx(self.handle, 9, info, 8)), "path_lease_metadata")
            require(not info[0] & 0x400 and (directory is None or bool(info[0] & 0x10) == directory), "redirected_path")
            buffer = ctypes.create_unicode_buffer(32768)
            size = self.api.GetFinalPathNameByHandleW(self.handle, buffer, len(buffer), 0)
            require(0 < size < len(buffer), "path_lease_resolution")
            actual = buffer.value.removeprefix("\\\\?\\")
            require(actual.rstrip("\\").casefold() == str(path).rstrip("\\").casefold(), "path_alias")
        except BaseException:
            self.close()
            raise

    def close(self):
        if self.handle is not None:
            self.api.CloseHandle(self.handle)
            self.handle = None

    def metadata(self):
        return windows_handle_metadata(self.handle, self.api)


class Directory:
    """Pin every ancestor and use descriptor-relative no-follow I/O on POSIX."""
    def __init__(self, path: Path, stack: ExitStack, cache: dict):
        self.path = absolute(path)
        self.parent = None if self.path.parent == self.path else get_dir(self.path.parent, stack, cache)
        before = self.stat_path()
        ordinary(before, True)
        self.fd = None
        self.lease = None
        if os.name == "posix":
            flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
            self.fd = os.open(str(self.path) if self.parent is None else self.path.name, flags,
                              dir_fd=None if self.parent is None else self.parent.fd)
            stack.callback(self.close)
            opened = os.fstat(self.fd)
        elif os.name == "nt":
            self.lease = WindowsLease(self.path, True)
            stack.callback(self.close)
            opened = self.stat_path()
        else:
            raise Rejected("identity_backend_unavailable")
        try:
            ordinary(opened, True)
            require(identity(opened) == identity(before), "directory_changed")
        except BaseException:
            self.close()
            raise
        self.ident = identity(opened)

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
        if self.lease is not None:
            self.lease.close()
            self.lease = None

    def stat_path(self):
        if self.parent is not None and os.name == "posix":
            return os.stat(self.path.name, dir_fd=self.parent.fd, follow_symlinks=False)
        return windows_path_metadata(self.path, True) if os.name == "nt" else self.path.lstat()

    def check(self):
        if self.parent is not None:
            self.parent.check()
        current = self.stat_path()
        ordinary(current, True)
        require(identity(current) == self.ident, "directory_changed")

    def stat(self, leaf: str):
        return (os.stat(leaf, dir_fd=self.fd, follow_symlinks=False) if os.name == "posix" else
                windows_path_metadata(self.path / leaf))

    def open(self, leaf: str, write: bool = False):
        self.check()
        flags = (os.O_WRONLY | os.O_CREAT | os.O_EXCL) if write else os.O_RDONLY
        if os.name == "posix":
            fd = os.open(leaf, flags | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=self.fd)
            return os.fdopen(fd, "wb" if write else "rb")
        if write:
            return (self.path / leaf).open("xb")
        import msvcrt
        lease = WindowsLease(self.path / leaf, False)
        try:
            fd = msvcrt.open_osfhandle(lease.handle, os.O_RDONLY | os.O_BINARY)
            lease.handle = None  # The CRT descriptor now owns the Windows handle.
            return os.fdopen(fd, "rb")
        finally:
            lease.close()

    def mkdir(self, leaf: str):
        self.check()
        if os.name == "posix":
            os.mkdir(leaf, 0o700, dir_fd=self.fd)
        else:
            (self.path / leaf).mkdir(mode=0o700)

    def entries(self):
        self.check()
        return os.scandir(self.fd if os.name == "posix" else self.path)

    def metadata(self):
        return os.fstat(self.fd) if os.name == "posix" else self.lease.metadata()


def get_dir(path: Path, stack: ExitStack, cache: dict) -> Directory:
    if path not in cache:
        require(len(cache) < 32768, "directory_limit")
        cache[path] = Directory(path, stack, cache)
    return cache[path]


class Inputs:
    def __init__(self, root: Path, stack: ExitStack, cache: dict):
        self.root, self.stack, self.cache = absolute(root), stack, cache
        get_dir(self.root, stack, cache).check()
        self.seen = {}

    def read(self, relative: str, expected: dict | None, limit: int, output=None, collect: bool = False):
        relative = name(relative)
        path = self.root / relative
        parent = get_dir(path.parent, self.stack, self.cache)
        before = parent.stat(path.name)
        ordinary(before, False)
        require(0 <= before.st_size <= limit, "input_size")
        require(expected is None or before.st_size == expected["bytes"], "input_binding")
        previous = self.seen.get(relative)
        require(previous is None or previous == snapshot(before), "input_identity_changed")
        sha, count, chunks = hashlib.sha256(), 0, []
        with parent.open(path.name) as stream:
            opened = fstat_metadata(stream.fileno())
            ordinary(opened, False)
            require(snapshot(opened) == snapshot(before), "input_identity_changed")
            while chunk := stream.read(CHUNK):
                count += len(chunk)
                require(count <= limit, "input_size")
                require(expected is None or count <= expected["bytes"], "input_changed")
                sha.update(chunk)
                if output is not None:
                    output.write(chunk)
                if collect:
                    chunks.append(chunk)
            require(snapshot(fstat_metadata(stream.fileno())) == snapshot(before), "input_changed")
        parent.check()
        require(snapshot(parent.stat(path.name)) == snapshot(before), "input_identity_changed")
        actual = {"bytes": count, "sha256": sha.hexdigest()}
        require(count == before.st_size, "input_changed")
        require(expected is None or actual == blob(expected), "input_binding")
        self.seen[relative] = snapshot(before)
        return actual, b"".join(chunks) if collect else None


class Output:
    """Fresh destination is staging; failed staging is conservatively retained."""
    def __init__(self, destination: Path, stack: ExitStack, cache: dict):
        self.root, self.stack, self.cache = absolute(destination), stack, cache
        self.created = []
        self.files = {}
        self.identities = {}
        self.marker_fd = None
        parent = get_dir(self.root.parent, stack, cache)
        parent.mkdir(self.root.name)  # Exclusive: existing files/directories are never reused.
        self.created.append((parent, self.root.name, True, identity(parent.stat(self.root.name))))
        try:
            self.directory = get_dir(self.root, stack, cache)
        except BaseException:
            self.cleanup()
            raise

    def parent(self, relative: str) -> tuple[Directory, str]:
        parts = name(relative).split("/")
        parent = self.directory
        for part in parts[:-1]:
            path = parent.path / part
            if path not in self.cache:
                parent.mkdir(part)
                self.created.append((parent, part, True, identity(parent.stat(part))))
            parent = get_dir(path, self.stack, self.cache)
        return parent, parts[-1]

    def write(self, relative: str, body: bytes | None = None, source: Inputs | None = None,
              descriptor: dict | None = None, limit: int = MAX_FILE) -> dict:
        parent, leaf = self.parent(relative)
        with parent.open(leaf, True) as stream:
            opened = fstat_metadata(stream.fileno())
            self.created.append((parent, leaf, False, identity(opened)))
            ordinary(opened, False)
            if relative == "COMPLETE":
                require(self.marker_fd is None, "completion_marker_duplicate")
                # Retain the actual created object, not merely its pathname.
                # Refusal can invalidate this marker without unlinking a
                # replacement installed by a concurrent writer.
                self.marker_fd = os.dup(stream.fileno())
                self.stack.callback(os.close, self.marker_fd)
            if source is not None:
                actual, _ = source.read(descriptor["path"], descriptor, limit, output=stream)
            else:
                require(body is not None and len(body) <= MAX_JSON, "metadata_size")
                stream.write(body)
                actual = {"bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()}
            stream.flush()
            os.fsync(stream.fileno())
            finished = fstat_metadata(stream.fileno())
            ordinary(finished, False)
            require(identity(finished) == identity(opened) and finished.st_size == actual["bytes"], "output_changed")
            own = snapshot(finished)
        parent.check()
        current = parent.stat(leaf)
        ordinary(current, False)
        require(snapshot(current) == own, "output_changed")
        self.files[relative] = actual
        self.identities[relative] = own
        return actual

    def verify_namespace(self):
        """Bounded observed snapshot; this does not exclude concurrent writers."""
        directories = {}
        for parent, leaf, directory, ident in self.created:
            if directory:
                relative = (parent.path / leaf).relative_to(self.root).as_posix()
                directories["" if relative == "." else relative] = ident
        require(set(self.files) == set(self.identities), "output_namespace_records")
        require(len(directories) + len(self.files) <= MAX_OUTPUT_ENTRIES, "output_namespace_limit")
        seen_dirs, seen_files = set(), set()
        pending = [("", self.directory)]
        count = 1  # Root plus every enumerated entry, monotonically bounded.
        while pending:
            relative, directory = pending.pop()
            directory.check()
            before = directory.metadata()
            ordinary(before, True)
            require(identity(before) == directories[relative], "output_directory_identity_changed")
            require(relative not in seen_dirs, "output_namespace_duplicate")
            seen_dirs.add(relative)
            names = set()
            with directory.entries() as entries:
                for entry in entries:
                    child = name(entry.name if not relative else relative + "/" + entry.name)
                    count += 1
                    require(count <= MAX_OUTPUT_ENTRIES, "output_namespace_limit")
                    require(entry.name not in names, "output_namespace_duplicate")
                    names.add(entry.name)
                    current = directory.stat(entry.name)  # Never follows links.
                    if child in directories:
                        ordinary(current, True)
                        require(identity(current) == directories[child], "output_directory_identity_changed")
                        pending.append((child, get_dir(directory.path / entry.name, self.stack, self.cache)))
                    elif child in self.identities:
                        ordinary(current, False)
                        require(snapshot(current) == self.identities[child], "output_identity_changed")
                        seen_files.add(child)
                    else:
                        raise Rejected("output_namespace_unlisted")
            require(snapshot(directory.metadata()) == snapshot(before), "output_namespace_changed")
            directory.check()
        require(seen_dirs == set(directories) and seen_files == set(self.files), "output_namespace_missing")
        if self.marker_fd is not None:
            require(snapshot(fstat_metadata(self.marker_fd)) == self.identities["COMPLETE"], "completion_marker_changed")

    def cleanup(self):
        # A POSIX unlink cannot target a retained inode. A stat followed by
        # unlink can delete a foreign replacement, even through a pinned parent.
        # No backend currently proves exclusion for every output entry, so
        # retain the complete failed staging tree on every platform. Invalidate
        # only our marker descriptor; never delete or truncate a checked path.
        if self.marker_fd is not None:
            try:
                os.ftruncate(self.marker_fd, 0)
                os.fsync(self.marker_fd)
            except OSError as error:
                raise Rejected("incomplete_marker_invalidation_failed") from error


def blob(row: dict) -> dict:
    return {key: row[key] for key in ("bytes", "sha256")}


def descriptor(row, limit: int = MAX_FILE) -> dict:
    require(isinstance(row, dict) and set(row) == {"path", "bytes", "sha256"}, "descriptor")
    name(row["path"])
    require(type(row["bytes"]) is int and 0 <= row["bytes"] <= limit and
            isinstance(row["sha256"], str) and HASH.fullmatch(row["sha256"]) is not None, "descriptor_binding")
    return row


def unique_paths(rows: list, key: str = "path") -> None:
    seen = []
    for row in rows:
        seen.append(name(row[key]).casefold())
    require(len(set(seen)) == len(seen), "duplicate_path")
    values = set(seen)
    for current in values:
        parts = current.split("/")
        require(not any("/".join(parts[:index]) in values for index in range(1, len(parts))), "path_collision")


def parse(raw: bytes) -> dict:
    def unique(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value, "json_duplicate")
            value[key] = item
        return value
    require(len(raw) <= MAX_JSON, "json_size")
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique,
                           parse_constant=lambda _: (_ for _ in ()).throw(Rejected("json_constant")))
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        raise Rejected("json_encoding_or_structure") from error
    require(isinstance(value, dict), "json_object")
    return value


def json_bytes(value: dict) -> bytes:
    body = (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")
    require(len(body) <= MAX_JSON, "json_size")
    return body


def manifest(value: dict) -> dict:
    required = {"schema", "release_id", "binding", "artifacts", "notice_manifest", "notices", "catalog"}
    require(required <= set(value) <= required | {"release_documents", "external_dependencies"}
            and value["schema"] == "vw-release-assembly-v1", "manifest_schema")
    require(isinstance(value["release_id"], str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,79}", value["release_id"]), "release_id")
    descriptor(value["binding"], MAX_JSON)
    descriptor(value["notice_manifest"], MAX_JSON)
    require(value["notice_manifest"]["path"] == "third_party/LICENSES", "notice_manifest_path")
    require(isinstance(value["notices"], list) and 0 < len(value["notices"]) <= 512, "notice_count")
    for row in value["notices"]:
        descriptor(row, 4 * CHUNK)
        require(row["path"].startswith("third_party/notices/") and len(row["path"].split("/")) == 3 and row["bytes"] > 0, "notice_path")
    require(sum(row["bytes"] for row in value["notices"]) <= 32 * CHUNK, "notice_total")
    require(isinstance(value["artifacts"], list) and 0 < len(value["artifacts"]) <= MAX_ARTIFACTS, "artifact_count")
    for row in value["artifacts"]:
        require(isinstance(row, dict) and set(row) == {"path", "bytes", "sha256", "destination", "kind"}, "artifact_descriptor")
        descriptor({key: row[key] for key in ("path", "bytes", "sha256")})
        name(row["destination"])
        require(row["destination"].startswith("artifacts/") and row["bytes"] > 0 and
                row["kind"] in ("opaque", "external-package", "apk", "desktop-file"), "artifact_kind_or_destination")
        lower = row["path"].lower()
        require((row["kind"] == "apk") == lower.endswith(".apk"), "apk_kind")
        require((row["kind"] == "external-package") == lower.endswith(".msi"), "msi_kind")
        require((row["kind"] == "desktop-file") == row["path"].startswith(DESKTOP), "desktop_kind")
    require(sum(row["bytes"] for row in value["artifacts"]) <= MAX_TOTAL, "artifact_total")
    if value["catalog"] is not None:
        descriptor(value["catalog"], 32 * 1024)
        require(any(row["kind"] == "desktop-file" for row in value["artifacts"]), "unused_catalog")
    documents = value.get("release_documents", dict.fromkeys(DOCUMENTS))
    require(isinstance(documents, dict) and set(documents) == set(DOCUMENTS), "release_documents")
    for row in documents.values():
        if row is not None:
            descriptor(row, MAX_JSON)
            require(row["bytes"] > 0, "release_document_empty")
    dependencies = value.get("external_dependencies", [])
    require(isinstance(dependencies, list) and len(dependencies) <= 64, "external_dependencies")
    for row in dependencies:
        require(isinstance(row, dict) and set(row) == {"id", "reason"} and
                isinstance(row["id"], str) and re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,79}", row["id"]) and
                isinstance(row["reason"], str) and 0 < len(row["reason"]) <= 512 and
                all(32 <= ord(ch) < 127 for ch in row["reason"]), "external_dependency")
    require(len({row["id"] for row in dependencies}) == len(dependencies), "external_dependency_duplicate")
    all_inputs = [value["binding"], value["notice_manifest"], *value["notices"], *value["artifacts"]]
    all_inputs.extend(row for row in documents.values() if row is not None)
    if value["catalog"] is not None:
        all_inputs.append(value["catalog"])
    unique_paths(all_inputs)
    unique_paths(value["artifacts"], "destination")
    return value


def binding(value: dict, artifacts: list) -> list:
    require(set(value) == {"schema", "source_revision", "sources", "artifacts"} and
            value["schema"] == "vw-release-source-binding-v1", "binding_schema")
    require(isinstance(value["source_revision"], str) and re.fullmatch(r"[0-9a-f]{40}", value["source_revision"]), "source_revision")
    require(isinstance(value["sources"], list) and 0 < len(value["sources"]) <= MAX_SOURCES, "source_count")
    for row in value["sources"]:
        descriptor(row, MAX_SOURCE_FILE)
    unique_paths(value["sources"])
    require(sum(row["bytes"] for row in value["sources"]) <= MAX_SOURCE_TOTAL, "source_total")
    require(isinstance(value["artifacts"], list) and len(value["artifacts"]) == len(artifacts), "binding_artifacts")
    for row in value["artifacts"]:
        descriptor(row)
    unique_paths(value["artifacts"])
    wanted = [{key: row[key] for key in ("path", "bytes", "sha256")} for row in artifacts]
    require(sorted(value["artifacts"], key=lambda x: x["path"]) == sorted(wanted, key=lambda x: x["path"]), "binding_artifacts")
    return value["sources"]


def validate_packages(root: Path, output: Output, spec: dict, declarations: dict, personal, apk) -> dict:
    results = {}
    desktop = [row for row in spec["artifacts"] if row["kind"] == "desktop-file"]
    if desktop:
        require(any(row["path"] == "apps/android/build/outputs/apk/debug/android-debug.apk" and
                    row["kind"] == "apk" for row in spec["artifacts"]), "personal_package_requires_explicit_apk")
        require(root == Path(__file__).absolute().parents[2], "desktop_validator_requires_tool_checkout")
        catalog = spec["catalog"]
        built = personal.r.check(root)
        closure = dict(built["source"])
        closure.update({"target/debug/" + key: value for key, value in built["distribution"]["native_inputs"].items()})
        closure.update({key: personal.r.digest(root / key) for key in (personal.r.BEGIN, personal.r.BUILT)})
        require(all(declarations.get(key) == value for key, value in closure.items()), "desktop_requires_explicit_build_inputs")
        report, _ = personal.verify(root, None if catalog is None else root / catalog["path"],
                                    None if catalog is None else catalog["sha256"])
        expected = built["distribution"]["files"]
        require({row["path"].removeprefix(DESKTOP): blob(row) for row in desktop} == expected,
                "desktop_requires_complete_admitted_image")
        prefixes = set()
        for row in desktop:
            tail = row["path"].removeprefix(DESKTOP)
            require(row["destination"].endswith("/" + tail), "desktop_layout")
            prefixes.add(row["destination"][:-len(tail)])
        require(len(prefixes) == 1, "desktop_layout")
        results["personal_package"] = report
    for row in spec["artifacts"]:
        if row["kind"] == "apk":
            report = apk.verify_apk(output.root / row["destination"])
            require(report["apk_sha256"] == row["sha256"] and report["apk_bytes"] == row["bytes"], "apk_binding")
            results[row["destination"]] = report
    return results


def assemble(root: Path, manifest_name: str, expected_sha256: str, destination: Path) -> dict:
    require(isinstance(expected_sha256, str) and HASH.fullmatch(expected_sha256), "manifest_digest")
    root, destination = absolute(root), absolute(destination)
    # Outputs can live in .local, but never underneath any explicitly read input
    # directory (checked again once the complete source inventory is known).
    require(root != destination, "output_root")
    output = None
    cache = {}
    stack = ExitStack()
    try:
        source = Inputs(root, stack, cache)
        actual, raw = source.read(manifest_name, None, MAX_JSON, collect=True)
        require(actual["sha256"] == expected_sha256, "manifest_binding")
        spec = manifest(parse(raw))
        _, proof_raw = source.read(spec["binding"]["path"], spec["binding"], MAX_JSON, collect=True)
        proof = parse(proof_raw)
        sources = binding(proof, spec["artifacts"])
        documents = spec.get("release_documents", dict.fromkeys(DOCUMENTS))
        all_rows = [*sources, spec["binding"], spec["notice_manifest"], *spec["notices"], *spec["artifacts"]]
        all_rows.extend(row for row in documents.values() if row is not None)
        if spec["catalog"] is not None:
            all_rows.append(spec["catalog"])
        all_rows.append({"path": manifest_name, **actual})
        require(len(all_rows) <= MAX_SOURCES + MAX_ARTIFACTS + 518, "input_count")
        declarations = {}
        folds = {}
        for row in all_rows:
            path = root / row["path"]
            require(destination != path and destination not in path.parents and path not in destination.parents, "output_overlaps_input")
            require(row["path"] not in declarations or declarations[row["path"]] == blob(row), "inconsistent_input_binding")
            folded = row["path"].casefold()
            require(folded not in folds or folds[folded] == row["path"], "input_case_alias")
            folds[folded] = row["path"]
            declarations[row["path"]] = blob(row)
        for row in sources:
            source.read(row["path"], row, MAX_SOURCE_FILE)
        if spec["catalog"] is not None:
            source.read(spec["catalog"]["path"], spec["catalog"], 32 * 1024)
        output = Output(destination, stack, cache)
        output.write("release-manifest.json", raw)
        output.write("source-binding.json", proof_raw)
        prerequisites = {}
        for role, delivered in DOCUMENTS.items():
            row = documents[role]
            if row is None:
                prerequisites[role] = {"status": "missing-release-prerequisite"}
            else:
                output.write(delivered, source=source, descriptor=row, limit=MAX_JSON)
                prerequisites[role] = {"status": "supplied-hash-verified-content-not-accepted",
                                       "input": row, "delivered_path": delivered}
        output.write("third_party/LICENSES", source=source, descriptor=spec["notice_manifest"], limit=MAX_JSON)
        for row in spec["notices"]:
            output.write(row["path"], source=source, descriptor=row, limit=4 * CHUNK)
        # This invokes the existing full notice metadata and exact-inventory gate
        # against the copied snapshot; inputs are not reinterpreted as licenses.
        _, notice_rows = PERSONAL.notices(output.root)
        for row in spec["artifacts"]:
            output.write(row["destination"], source=source, descriptor=row)
        package_results = validate_packages(root, output, spec, declarations, PERSONAL, APK)
        lines = ["Visual Workbench release assembly - actual notice inputs", "",
                 "License names and URLs are copied from third_party/LICENSES; no compatibility decision is made.", ""]
        for row in notice_rows:
            lines += [f"third_party/notices/{row['resource']} | {row['license']} | {row['bytes']} bytes | SHA256 {row['sha256']}", row["source_url"]]
        lines += ["", "Preserve all delivered runtime/legal files. Runtime legal files are in the artifact inventory.", ""]
        output.write("THIRD-PARTY-NOTICES.txt", "\n".join(lines).encode())
        # Recheck original identities and bytes after package validators, and
        # every copied output before publishing the inventory/COMPLETE marker.
        for relative, expected in declarations.items():
            source.read(relative, expected, max(MAX_SOURCE_FILE, expected["bytes"]))
        copied = Inputs(output.root, stack, cache)
        for relative, expected in output.files.items():
            path = output.root / relative
            require(snapshot(get_dir(path.parent, stack, cache).stat(path.name)) == output.identities[relative], "output_identity_changed")
            copied.read(relative, expected, max(MAX_JSON, expected["bytes"]))
        report = {"schema": "vw-release-inventory-v1", "status": "assembled-hash-verified",
                  "release_id": spec["release_id"], "manifest": actual, "source_revision_label": proof["source_revision"],
                  "source_binding": "exact declared current-source and artifact hashes; build causality and revision label are not authenticated",
                  "inputs": [{"path": key, **value} for key, value in sorted(declarations.items())],
                  "files": dict(sorted(output.files.items())), "notices": notice_rows, "package_checks": package_results,
                  "release_prerequisites": prerequisites, "release_ready": False,
                  "external_dependencies": spec.get("external_dependencies", []),
                  "output_namespace_entry_limit": MAX_OUTPUT_ENTRIES,
                  "output_namespace_check": "observed whole namespace before and after COMPLETE; concurrent mutation is not excluded",
                  "built": False, "signed_or_signature_verified": False, "installed_or_launched": False,
                  "device_or_physical_acceptance": False,
                  "external_package_validation": "MSI/opaque inputs receive hash binding only; root MSI admission is separate"}
        output.write("inventory.json", json_bytes(report))
        sums = "".join(f"{value['sha256']}  {key}\n" for key, value in sorted(output.files.items()))
        output.write("SHA256SUMS", sums.encode("ascii"))
        output.verify_namespace()
        output.write("COMPLETE", (output.files["inventory.json"]["sha256"] + "  inventory.json\n").encode("ascii"))
        output.verify_namespace()
        return report
    except BaseException:
        if output is not None:
            output.cleanup()
        raise
    finally:
        stack.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", required=True, help="exact relative input path; no globs")
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--destination", type=Path, required=True, help="fresh directory; parent must already exist")
    args = parser.parse_args()
    try:
        assemble(args.root, args.manifest, args.manifest_sha256, args.destination)
    except (Rejected, REPORTS.Rejected, APK.Rejected, OSError, ValueError, KeyError, TypeError,
            OverflowError, RecursionError, struct.error, zipfile.BadZipFile) as error:
        code = str(error) if isinstance(error, Rejected) else "input_or_validator_refused"
        print("RELEASE_ASSEMBLY_REJECTED:" + code)
        return 1
    print("RELEASE_ASSEMBLY_OK:assembled-hash-verified")
    return 0


REPORTS, PERSONAL, APK = validators()

if __name__ == "__main__":
    raise SystemExit(main())
