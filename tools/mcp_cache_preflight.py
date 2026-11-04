"""Read-only preflight for the packaged MCP server cache (blocker B8).

The 2026-10-20 owner-approved cache action stopped at ``pin_cache_ancestors``
with ``canonical_path_changed``. The read-only diagnosis showed the refusal at
the app-data child ``Visual Workbench``: the requested path had 6 components,
GetFinalPathNameByHandle returned 10, and both opens named the same object
(volume serial and 128-bit file id equal, no reparse tag). The inferred
cause (not proven by a retained receipt) is MSIX app-data virtualization: a
process inside a packaged app container (likely the agent desktop app that
launched the build) has ``%LOCALAPPDATA%\\<child>`` redirected to
``%LOCALAPPDATA%\\Packages\\<family>\\LocalCache\\Local\\<child>``.
The desktop app already admits exactly this one-child redirection in
``NativeCacheAnchor``; the ad-hoc helper did not.

This module mirrors that rule and adds three read-only pieces:

* ``classify_final_path`` accepts syntax-only (``\\\\?\\`` prefix, trailing
  separator), case-only and the single admitted MSIX app-data redirection,
  and only when the requested and canonical opens are the same file object.
  Reparse points, different objects, other redirections and unexpanded 8.3
  names still refuse.
  This is stricter than NativeCacheAnchor, which only requires the canonical
  anchor to start with LOCALAPPDATA and end in the anchor name.
* ``classify_processes`` refuses on any Workbench app or Workbench-only
  runtime binary by basename wherever installed, any image under the cache
  or an explicit packaged root, and node.exe descended from a Workbench or
  task-owned process. Other node.exe processes no longer block. A node.exe
  outside the cache running a cache script is not detectable by image path.
* ``plan_cache_entries`` lists task-owned cache entries and proposes which
  ones the owner could retire to free a slot. It never moves or deletes.

Nothing here renames or deletes cache content. The CLI prints a JSON plan
with paths rewritten relative to ``%LOCALAPPDATA%`` and writes the same
receipt under ``.local/mcp-cache-preflight/`` in this checkout.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import datetime
import json
import os
from pathlib import Path
import re
import sys
from typing import Callable, Iterable

ANCHOR_NAME = "Visual Workbench"
CACHE_BASE_NAME = "mcp-server-bundles-v1"
CACHE_CAPACITY = 8  # DesktopMcpBundle refuses a new key when 8 children exist.
OWNER_MARKER_PREFIX = "VisualWorkbench MCP server v1\n"
KEY_RE = re.compile(r"[0-9a-f]{64}\Z")
# PackageFamilyName = <Name 3..50 of [A-Za-z0-9.-]>_<13-char publisher id>.
PACKAGE_FAMILY_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9.\-]{2,49}_[0-9a-hjkmnp-tv-z]{13}\Z")
SHORT_NAME_RE = re.compile(r"[^~\\]{1,6}~[0-9]{1,6}(\.[^.\\]{0,3})?\Z")
# Launcher names from apps/desktop/build.gradle.kts (packageName) and
# mcp/package_runtime.py; runtime names from the bundle manifest contract.
APP_IMAGE_NAMES = frozenset(n.upper() for n in ("VisualWorkbench.exe", "VisualWorkbenchDev.exe"))
WORKBENCH_RUNTIME_NAMES = frozenset(n.upper() for n in (
    "vw-mcp-host.exe", "vw-mcp-package.exe", "vw-codex-host.exe", "vw-mcp.exe"))
RUNTIME_IMAGE_NAMES = frozenset({"NODE.EXE"}) | WORKBENCH_RUNTIME_NAMES | APP_IMAGE_NAMES
MAX_PATH_CHARS = 32767
MAX_COMPONENTS = 256


class PreflightRefused(Exception):
    """A fixed refusal code; never carries paths or native text."""

    def __init__(self, code: str, phase: str = "") -> None:
        super().__init__(code)
        self.code = code
        self.phase = phase


# --------------------------------------------------------------------------- paths

def split_dos_path(value: str) -> tuple[str, list[str]]:
    """Return (drive, components) for a drive-absolute DOS path.

    Accepts the ``\\\\?\\`` prefix, either separator and trailing separators.
    Refuses UNC/device namespaces, relative paths, ``.``/``..``, ADS colons
    and NUL, so a syntax difference can never hide a different location.
    """
    if not isinstance(value, str) or not value or len(value) > MAX_PATH_CHARS or "\0" in value:
        raise PreflightRefused("unsupported_namespace")
    text = value.replace("/", "\\")
    if text.startswith("\\\\?\\"):
        text = text[4:]
    elif text.startswith("\\\\"):
        raise PreflightRefused("unsupported_namespace")
    if len(text) < 3 or not ("A" <= text[0].upper() <= "Z") or text[1] != ":" or text[2] != "\\":
        raise PreflightRefused("unsupported_namespace")
    parts = [p for p in text[3:].split("\\") if p != ""]
    if "\\\\" in text[3:].rstrip("\\"):
        raise PreflightRefused("unsupported_namespace")
    if len(parts) > MAX_COMPONENTS or any(p in (".", "..") or ":" in p for p in parts):
        raise PreflightRefused("unsupported_namespace")
    return text[0].upper() + ":", parts


def _same_name(left: str, right: str) -> bool:
    # NTFS compares names with its upcase table; ordinal upper-casing is the
    # closest portable approximation and matches the app's ignoreCase check.
    return left.upper() == right.upper()


def _same_components(left: list[str], right: list[str]) -> bool:
    return len(left) == len(right) and all(_same_name(a, b) for a, b in zip(left, right))


def _starts_with(path: list[str], prefix: list[str]) -> bool:
    return len(path) >= len(prefix) and _same_components(path[:len(prefix)], prefix)


@dataclass(frozen=True)
class PathForm:
    form: str  # exact | syntax_only | case_only | msix_app_data_redirect
    package_family: str | None = None


def classify_final_path(requested: str, observed: str, *, local_app_data: str, same_object: bool,
                        reparse_point: bool, allow_redirect_child: str | None = None) -> PathForm:
    """Decide whether ``observed`` (a final handle path) names ``requested``.

    ``same_object`` must come from reopening ``observed`` and comparing the
    volume serial plus 128-bit file id with the handle opened at
    ``requested``. String equivalence alone is never sufficient.

    ``allow_redirect_child`` names the only immediate child of
    ``local_app_data`` whose subtree may be MSIX-redirected (mirrors
    NativeCacheAnchor). ``None`` admits no redirection.
    """
    if reparse_point:
        raise PreflightRefused("reparse_point")
    if not same_object:
        raise PreflightRefused("object_identity_changed")
    r_drive, r_parts = split_dos_path(requested)
    o_drive, o_parts = split_dos_path(observed)
    if any(SHORT_NAME_RE.match(p) for p in r_parts) and not _same_components(r_parts, o_parts):
        # The probe expands requested paths with GetLongPathNameW first; an
        # unexpanded alias that still differs is not guessed at.
        raise PreflightRefused("short_name_unexpanded")
    if r_drive == o_drive and r_parts == o_parts:
        raw_equal = requested == observed
        return PathForm("exact" if raw_equal else "syntax_only")
    if r_drive == o_drive and _same_components(r_parts, o_parts):
        return PathForm("case_only")
    if allow_redirect_child is not None:
        l_drive, l_parts = split_dos_path(local_app_data)
        redirect = _msix_redirect(l_drive, l_parts, r_drive, r_parts, o_drive, o_parts, allow_redirect_child)
        if redirect is not None:
            return PathForm("msix_app_data_redirect", redirect)
    raise PreflightRefused("canonical_path_changed")


def _msix_redirect(l_drive: str, l_parts: list[str], r_drive: str, r_parts: list[str], o_drive: str,
                   o_parts: list[str], child: str) -> str | None:
    if not (l_drive == r_drive == o_drive) or not l_parts or not _same_name(l_parts[-1], "Local"):
        return None
    if not _starts_with(r_parts, l_parts) or len(r_parts) <= len(l_parts):
        return None
    rest = r_parts[len(l_parts):]
    if not _same_name(rest[0], child) or _same_name(rest[0], "Packages"):
        return None
    if len(o_parts) != len(l_parts) + 4 + len(rest) or not _starts_with(o_parts, l_parts):
        return None
    packages, family, local_cache, local = o_parts[len(l_parts):len(l_parts) + 4]
    if not (_same_name(packages, "Packages") and _same_name(local_cache, "LocalCache") and _same_name(local, "Local")):
        return None
    if not PACKAGE_FAMILY_RE.match(family):
        return None
    if not _same_components(o_parts[len(l_parts) + 4:], rest):
        return None
    return family


# --------------------------------------------------------------------------- ancestor pinning

@dataclass(frozen=True)
class Opened:
    final_path: str
    volume_serial: int
    file_id: bytes
    attributes: int
    reparse_tag: int

    @property
    def identity(self) -> tuple[int, bytes]:
        return self.volume_serial, self.file_id

    @property
    def directory(self) -> bool:
        return bool(self.attributes & 0x10)

    @property
    def reparse_point(self) -> bool:
        return bool(self.attributes & 0x400) or self.reparse_tag != 0


class Probe:
    """Read-only file identity probe. ``open`` holds a lease until ``close``."""

    def long_path(self, path: str) -> str:
        raise NotImplementedError

    def open(self, path: str) -> Opened:
        raise NotImplementedError

    def reopen_identity(self, path: str) -> tuple[int, bytes]:
        raise NotImplementedError

    def close(self) -> None:
        raise NotImplementedError


@dataclass(frozen=True)
class PinnedComponent:
    label: str
    requested: str
    canonical: str
    form: str


@dataclass(frozen=True)
class PinnedChain:
    components: list[PinnedComponent]
    package_family: str | None

    @property
    def canonical(self) -> str:
        return self.components[-1].canonical


def _join(drive: str, parts: Iterable[str]) -> str:
    return drive + "\\" + "\\".join(parts)


def pin_cache_ancestors(local_app_data: str, below: list[str], probe: Probe, *,
                        redirect_child: str = ANCHOR_NAME, optional_tail: int = 0) -> PinnedChain | None:
    """Pin every component from the drive root to ``local_app_data\\below``.

    Components up to and including ``local_app_data`` must be exact, syntax
    or case equivalent. ``redirect_child`` (``below[0]``) may additionally be
    MSIX redirected, and every deeper component must keep that same package
    family. Returns ``None`` only when one of the last ``optional_tail``
    components is absent. The probe's leases stay open until ``probe.close``.
    """
    drive, l_parts = split_dos_path(probe.long_path(local_app_data))
    lad = _join(drive, l_parts)
    if any(p in ("", ".", "..") or "\\" in p or "/" in p or ":" in p for p in below):
        raise PreflightRefused("unsupported_namespace", "pin_cache_ancestors")
    chain = [_join(drive, l_parts[:i]) if i else drive + "\\" for i in range(len(l_parts) + 1)]
    chain += [_join(drive, l_parts + below[:i + 1]) for i in range(len(below))]
    first_optional = len(chain) - optional_tail
    pinned: list[PinnedComponent] = []
    family: str | None = None
    for index, requested in enumerate(chain):
        depth_below = index - len(l_parts)  # 0 = local_app_data, 1 = redirect child
        try:
            opened = probe.open(requested)
        except FileNotFoundError:
            if index >= first_optional:
                return None
            raise PreflightRefused("component_absent", "pin_cache_ancestors") from None
        except OSError:
            raise PreflightRefused("pin_failed", "pin_cache_ancestors") from None
        if not opened.directory:
            raise PreflightRefused("not_directory", "pin_cache_ancestors")
        try:
            same = probe.reopen_identity(opened.final_path) == opened.identity
        except OSError:
            same = False
        allow = redirect_child if depth_below >= 1 and _same_name(below[0], redirect_child) else None
        try:
            form = classify_final_path(requested, opened.final_path, local_app_data=lad, same_object=same,
                                       reparse_point=opened.reparse_point, allow_redirect_child=allow)
        except PreflightRefused as refused:
            raise PreflightRefused(refused.code, "pin_cache_ancestors") from None
        if form.form == "msix_app_data_redirect":
            if family is None and depth_below != 1:
                raise PreflightRefused("canonical_path_changed", "pin_cache_ancestors")
            if family is not None and (form.package_family or "").upper() != family.upper():
                raise PreflightRefused("canonical_path_changed", "pin_cache_ancestors")
            family = form.package_family
        elif family is not None:
            # Once the anchor is redirected, a descendant resolving back to the
            # unredirected namespace is a different location.
            raise PreflightRefused("canonical_path_changed", "pin_cache_ancestors")
        label = "volume_root" if index == 0 else ("local_app_data" if depth_below == 0 else
                                                  (f"below_{depth_below}" if depth_below > 0 else f"ancestor_{index}"))
        _, canonical_parts = split_dos_path(opened.final_path)
        pinned.append(PinnedComponent(label, requested, _join(drive, canonical_parts), form.form))
    return PinnedChain(pinned, family)


# --------------------------------------------------------------------------- consumer guard

@dataclass(frozen=True)
class ProcessRow:
    pid: int
    ppid: int
    name: str
    executable: str | None  # None when the image path is unreadable
    created: int | None  # FILETIME ticks; None when unreadable


def _under(path: str, root: str) -> bool:
    try:
        p_drive, p_parts = split_dos_path(path)
        r_drive, r_parts = split_dos_path(root)
    except PreflightRefused:
        return False
    return p_drive == r_drive and _starts_with(p_parts, r_parts)


def owned_tree(rows: list[ProcessRow], roots: Iterable[int]) -> set[int]:
    """PIDs descended from ``roots`` (inclusive). A child is linked only when it
    was created no earlier than its parent, which rejects PID reuse."""
    by_pid = {row.pid: row for row in rows}
    owned = {pid for pid in roots if pid in by_pid}
    changed = True
    while changed:
        changed = False
        for row in rows:
            if row.pid in owned or row.ppid not in owned or row.ppid == row.pid:
                continue
            parent = by_pid[row.ppid]
            if row.created is not None and parent.created is not None and row.created < parent.created:
                continue
            owned.add(row.pid)
            changed = True
    return owned


def _parseable(path: str) -> bool:
    try:
        split_dos_path(path)
    except PreflightRefused:
        return False
    return True


def classify_processes(rows: list[ProcessRow], *, owned_roots: Iterable[int],
                       packaged_roots: Iterable[str], self_pid: int | None = None,
                       excluded_pids: Iterable[int] = ()) -> dict:
    """Scope the consumer guard to Workbench/task-owned processes.

    Refuses (``conflicts`` non-empty) when, for any process not in
    ``excluded_pids``:
    * its image lies under a packaged root (the cache base in both its
      requested and canonical forms, or an explicit ``--packaged-root``);
    * its image basename is the Workbench app (VisualWorkbench.exe /
      VisualWorkbenchDev.exe), wherever it is installed;
    * its image basename is a Workbench-only runtime binary (vw-mcp-host.exe,
      vw-mcp-package.exe, vw-codex-host.exe, vw-mcp.exe), wherever it is;
    * it is node.exe descended from a task-owned root or from a running
      Workbench app process;
    * it is a runtime-named process whose image is unreadable or not a
      drive path (ambiguous).
    Other node.exe processes are counted and ignored. Limitation: a node.exe
    installed outside the cache that executes a script from the cache is not
    detectable by image path and is not reported unless it descends from a
    Workbench or task-owned process.
    """
    roots = [r for r in packaged_roots if r]
    excluded = set(excluded_pids)
    if self_pid is not None:
        excluded.add(self_pid)
    live = [row for row in rows if row.pid not in excluded]
    app_pids = [row.pid for row in live if row.name.upper() in APP_IMAGE_NAMES]
    owned = owned_tree(rows, owned_roots)
    app_tree = owned_tree(rows, app_pids)
    conflicts: list[dict] = []
    unrelated_node = 0
    for row in live:
        name = row.name.upper()
        runtime_named = name in RUNTIME_IMAGE_NAMES
        readable = row.executable is not None and _parseable(row.executable)
        if readable and any(_under(row.executable, root) for root in roots):
            reason = "packaged_runtime_in_use"
        elif name in APP_IMAGE_NAMES:
            reason = "workbench_app_running"
        elif name in WORKBENCH_RUNTIME_NAMES:
            reason = "workbench_runtime_named_process"
        elif runtime_named and row.pid in owned:
            reason = "task_owned_runtime_alive"
        elif runtime_named and row.pid in app_tree:
            reason = "workbench_app_child_runtime"
        elif runtime_named and not readable:
            reason = "runtime_named_image_ambiguous"
        else:
            if runtime_named:
                unrelated_node += 1
            continue
        conflicts.append({"pid": row.pid, "name": row.name, "reason": reason})
    return {
        "status": "refused" if conflicts else "clear",
        "conflicts": conflicts,
        "workbench_app_running": bool(app_pids),
        "owned_tree_size": len(owned),
        "excluded_pid_count": len(excluded - ({self_pid} if self_pid is not None else set())),
        "unrelated_runtime_named_processes_ignored": unrelated_node,
        "processes_changed": False,
        "limitation": "node.exe outside the cache running a cache script is not detectable by image path",
    }


# --------------------------------------------------------------------------- dry-run listing

@dataclass(frozen=True)
class CacheEntry:
    name: str
    plain_directory: bool
    owner_marker_valid: bool
    ready_present: bool
    runtime_present: bool
    unexpected_children: bool
    modified: float


def scan_cache(base: Path) -> list[CacheEntry]:
    """Read-only listing. Reads at most 256 bytes of each ``owner`` marker;
    never opens ``ready`` (the manifest copy) or any payload."""
    entries: list[CacheEntry] = []
    with os.scandir(base) as children:
        for index, child in enumerate(children):
            if index >= 64:
                raise PreflightRefused("cache_listing_unbounded", "scan_cache")
            st = child.stat(follow_symlinks=False)
            attributes = getattr(st, "st_file_attributes", 0)
            plain = child.is_dir(follow_symlinks=False) and not (attributes & 0x400)
            owner_valid = ready = runtime = unexpected = False
            if plain:
                names: set[str] = set()
                with os.scandir(child.path) as inner:
                    for count, item in enumerate(inner):
                        if count >= 8:
                            unexpected = True
                            break
                        names.add(item.name)
                unexpected = unexpected or not names <= {"owner", "ready", "runtime"}
                ready = "ready" in names
                runtime = "runtime" in names
                if "owner" in names:
                    owner_valid = _owner_valid(Path(child.path) / "owner", child.name)
            entries.append(CacheEntry(child.name, plain, owner_valid, ready, runtime, unexpected, st.st_mtime))
    return entries


def _owner_valid(path: Path, key: str) -> bool:
    try:
        st = path.lstat()
        if not (st.st_mode & 0o170000 == 0o100000) or getattr(st, "st_file_attributes", 0) & 0x400:
            return False
        with path.open("rb") as handle:
            data = handle.read(257)
    except OSError:
        return False
    return KEY_RE.match(key) is not None and data == (OWNER_MARKER_PREFIX + key + "\n").encode("ascii")


def plan_cache_entries(entries: list[CacheEntry], *, requested_key: str | None, in_use: set[str],
                       capacity: int = CACHE_CAPACITY, workbench_running: bool = False) -> dict:
    """Classify entries and propose (never perform) the minimum retirement.

    Only entries with a valid task owner marker, a plain directory and no
    unexpected children are candidates; the requested key and anything a
    live process uses are always kept. Incomplete entries (no ``ready``)
    are proposed first, then the oldest complete ones. While any Workbench
    app process runs, an incomplete entry may be a live extraction and is
    never proposed. A requested key that exists but is not complete needs
    owner action: the app refuses it at ``bundle_markers`` and never
    re-extracts it.
    """
    if requested_key is not None and not KEY_RE.match(requested_key):
        raise PreflightRefused("requested_key_invalid", "plan")
    rows = []
    candidates = []
    requested_damaged = False
    for entry in sorted(entries, key=lambda e: (e.modified, e.name)):
        task_owned = (entry.plain_directory and KEY_RE.match(entry.name) is not None and entry.owner_marker_valid
                      and entry.runtime_present and not entry.unexpected_children)
        if entry.name == requested_key:
            if task_owned and entry.ready_present:
                state = "requested_current_keep"
            else:
                state = "requested_current_incomplete_owner_action"
                requested_damaged = True
        elif not task_owned:
            state = "not_task_owned_keep"
        elif entry.name in in_use:
            state = "in_use_keep"
        elif not entry.ready_present and workbench_running:
            state = "incomplete_possible_live_extraction_keep"
        elif not entry.ready_present:
            state = "incomplete_task_owned"
        else:
            state = "complete_task_owned"
        row = {"key": entry.name, "state": state, "ready_present": entry.ready_present,
               "modified_utc": _utc(entry.modified)}
        rows.append(row)
        if state in ("incomplete_task_owned", "complete_task_owned"):
            candidates.append(row)
    candidates.sort(key=lambda r: (r["state"] != "incomplete_task_owned", r["modified_utc"], r["key"]))
    requested_present = requested_key is not None and any(e.name == requested_key for e in entries)
    slots_needed = 0 if requested_present else max(0, len(entries) - (capacity - 1))
    proposal = candidates[:slots_needed]
    return {
        "mode": "dry_run",
        "mutation_performed": False,
        "capacity": capacity,
        "entry_count": len(entries),
        "requested_key_present": requested_present,
        "slots_needed": slots_needed,
        "satisfiable_from_task_owned_candidates": len(candidates) >= slots_needed,
        "entries": rows,
        "proposed_for_owner_review": [r["key"] for r in proposal],
        "workbench_app_running": workbench_running,
        "owner_action_required": requested_damaged or slots_needed > 0,
        "owner_action": (
            "the requested key exists but is incomplete or not task-owned; the app will refuse it at "
            "bundle_markers. Close Workbench, review that entry and move (do not delete) it out of the "
            "cache, then rerun this preflight" if requested_damaged else
            "none" if slots_needed == 0 else
            "review each proposed key; close Workbench/agent owners; move (do not delete) the "
            "approved entries out of the cache, then rerun this preflight before the smoke"),
    }


def _utc(value: float) -> str:
    return datetime.datetime.fromtimestamp(value, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# --------------------------------------------------------------------------- Windows probes

class WindowsProbe(Probe):  # pragma: no cover - exercised by the Windows-only tests
    """ctypes probe: FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES with read/write
    sharing and no delete sharing, so pinned components cannot be renamed or removed while
    the preflight holds them. Nothing is written."""

    def __init__(self) -> None:
        import ctypes
        from ctypes import wintypes
        self._ctypes = ctypes
        self._k = ctypes.WinDLL("kernel32", use_last_error=True)
        self._k.CreateFileW.restype = wintypes.HANDLE
        self._k.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                                        wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        self._k.GetFinalPathNameByHandleW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR, wintypes.DWORD, wintypes.DWORD]
        self._k.GetFinalPathNameByHandleW.restype = wintypes.DWORD
        self._k.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self._k.GetFileInformationByHandleEx.restype = wintypes.BOOL
        self._k.GetLongPathNameW.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD]
        self._k.GetLongPathNameW.restype = wintypes.DWORD
        self._k.CloseHandle.argtypes = [wintypes.HANDLE]
        self._handles: list[int] = []

    def _create(self, path: str, share: int, access: int = 0x80) -> int:
        native = path if len(path) < 260 or path.startswith("\\\\?\\") else "\\\\?\\" + path
        handle = self._k.CreateFileW(native, access, share, None, 3,
                                     0x02000000 | 0x00200000, None)
        if handle in (None, -1, 2 ** 64 - 1, 2 ** 32 - 1):
            error = self._ctypes.get_last_error()
            if error in (2, 3):
                raise FileNotFoundError(path)
            raise OSError(error, "open refused")
        return handle

    def _identity(self, handle: int) -> tuple[int, bytes, int, int]:
        ctypes = self._ctypes

        class FileIdInfo(ctypes.Structure):
            _fields_ = [("volume", ctypes.c_ulonglong), ("file_id", ctypes.c_ubyte * 16)]

        class AttributeTag(ctypes.Structure):
            _fields_ = [("attributes", ctypes.c_ulong), ("tag", ctypes.c_ulong)]

        ident, tag = FileIdInfo(), AttributeTag()
        if not self._k.GetFileInformationByHandleEx(handle, 18, ctypes.byref(ident), ctypes.sizeof(ident)):
            raise OSError(ctypes.get_last_error(), "file id unavailable")
        if not self._k.GetFileInformationByHandleEx(handle, 9, ctypes.byref(tag), ctypes.sizeof(tag)):
            raise OSError(ctypes.get_last_error(), "attributes unavailable")
        return ident.volume, bytes(ident.file_id), tag.attributes, tag.tag

    def long_path(self, path: str) -> str:
        buffer = self._ctypes.create_unicode_buffer(MAX_PATH_CHARS)
        length = self._k.GetLongPathNameW(path, buffer, MAX_PATH_CHARS)
        if length == 0 or length >= MAX_PATH_CHARS:
            raise PreflightRefused("long_path_unavailable", "pin_cache_ancestors")
        return buffer.value

    def open(self, path: str) -> Opened:
        # FILE_LIST_DIRECTORY makes the open take part in share checks; an
        # attributes-only handle would not stop a rename or delete.
        handle = self._create(path, 0x1 | 0x2, 0x80 | 0x1)
        self._handles.append(handle)
        buffer = self._ctypes.create_unicode_buffer(MAX_PATH_CHARS)
        length = self._k.GetFinalPathNameByHandleW(handle, buffer, MAX_PATH_CHARS, 0)
        if length == 0 or length >= MAX_PATH_CHARS:
            raise OSError(self._ctypes.get_last_error(), "final path unavailable")
        volume, file_id, attributes, tag = self._identity(handle)
        return Opened(buffer.value, volume, file_id, attributes, tag)

    def reopen_identity(self, path: str) -> tuple[int, bytes]:
        handle = self._create(path, 0x1 | 0x2 | 0x4)
        try:
            volume, file_id, _, _ = self._identity(handle)
            return volume, file_id
        finally:
            self._k.CloseHandle(handle)

    def close(self) -> None:
        while self._handles:
            self._k.CloseHandle(self._handles.pop())


def windows_processes() -> list[ProcessRow]:  # pragma: no cover - Windows only
    """Toolhelp snapshot plus PROCESS_QUERY_LIMITED_INFORMATION image/time reads."""
    import ctypes
    from ctypes import wintypes
    k = ctypes.WinDLL("kernel32", use_last_error=True)

    class Entry(ctypes.Structure):
        _fields_ = [("size", wintypes.DWORD), ("usage", wintypes.DWORD), ("pid", wintypes.DWORD),
                    ("heap", ctypes.c_void_p), ("module", wintypes.DWORD), ("threads", wintypes.DWORD),
                    ("ppid", wintypes.DWORD), ("priority", ctypes.c_long), ("flags", wintypes.DWORD),
                    ("exe", ctypes.c_wchar * 260)]

    k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    k.OpenProcess.restype = wintypes.HANDLE
    k.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    k.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR,
                                             ctypes.POINTER(wintypes.DWORD)]
    k.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(ctypes.c_ulonglong)] * 4
    k.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    k.CloseHandle.argtypes = [wintypes.HANDLE]
    k.Process32FirstW.argtypes = k.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(Entry)]
    snapshot = k.CreateToolhelp32Snapshot(0x2, 0)
    if snapshot in (None, wintypes.HANDLE(-1).value):
        raise PreflightRefused("process_snapshot_unavailable", "consumer_guard")
    rows: list[ProcessRow] = []
    try:
        entry = Entry()
        entry.size = ctypes.sizeof(Entry)
        ok = k.Process32FirstW(snapshot, ctypes.byref(entry))
        while ok:
            executable = created = None
            gone = False
            handle = k.OpenProcess(0x1000, False, entry.pid)
            if not handle and ctypes.get_last_error() == 87:
                # ERROR_INVALID_PARAMETER: the PID exited after the snapshot.
                # Access denied (5) and other errors stay ambiguous rows.
                gone = True
            if handle:
                try:
                    code = wintypes.DWORD()
                    if k.GetExitCodeProcess(handle, ctypes.byref(code)) and code.value != 259:
                        gone = True  # exited; only a lingering handle keeps the object
                    buffer = ctypes.create_unicode_buffer(MAX_PATH_CHARS)
                    size = wintypes.DWORD(MAX_PATH_CHARS)
                    if k.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
                        executable = buffer.value
                    times = [ctypes.c_ulonglong() for _ in range(4)]
                    if k.GetProcessTimes(handle, *[ctypes.byref(t) for t in times]):
                        created = times[0].value
                finally:
                    k.CloseHandle(handle)
            if not gone:
                rows.append(ProcessRow(entry.pid, entry.ppid, entry.exe, executable, created))
            ok = k.Process32NextW(snapshot, ctypes.byref(entry))
        # Only ERROR_NO_MORE_FILES ends a complete walk; any other failure
        # means rows are missing, which must not read as "clear".
        _require_complete_walk(ctypes.get_last_error(), len(rows))
    finally:
        k.CloseHandle(snapshot)
    return rows


def _require_complete_walk(last_error: int, row_count: int) -> None:
    if last_error != 18 or row_count == 0:
        raise PreflightRefused("process_snapshot_incomplete", "consumer_guard")


# --------------------------------------------------------------------------- CLI

def _redact(path: str, lad_forms: list[str]) -> str:
    for form in lad_forms:
        if _under(path, form):
            _, p_parts = split_dos_path(path)
            _, l_parts = split_dos_path(form)
            return "%LOCALAPPDATA%" + "".join("\\" + p for p in p_parts[len(l_parts):])
    return "<outside %LOCALAPPDATA%>"


def _cache_key_of(executable: str, cache_forms: list[str]) -> str | None:
    for form in cache_forms:
        if not _under(executable, form):
            continue  # _under is False for any unparseable path
        exe_parts, base_parts = split_dos_path(executable)[1], split_dos_path(form)[1]
        if len(exe_parts) > len(base_parts) + 1:
            return exe_parts[len(base_parts)]
    return None


def run(local_app_data: str, requested_key: str | None, owned_roots: list[int], packaged_roots: list[str],
        probe: Probe, processes: Callable[[], list[ProcessRow]], scan: Callable[[Path], list[CacheEntry]] = scan_cache,
        self_pid: int | None = None, excluded_pids: Iterable[int] = ()) -> dict:
    """Every failure becomes a structured refusal receipt; nothing escapes as
    a traceback that could be mistaken for an inconclusive pass."""
    receipt: dict = {"kind": "mcp-cache-preflight-dry-run", "schema": 2, "mutation_performed": False,
                     "smoke_executed": False}
    phase = "pin_cache_ancestors"
    try:
        chain = pin_cache_ancestors(local_app_data, [ANCHOR_NAME, CACHE_BASE_NAME], probe, optional_tail=2)
        if chain is None:
            receipt.update(status="cache_absent_in_this_namespace", phase=phase,
                           note="no Visual Workbench cache is visible to this process; a launcher in a "
                                "different package container sees a different cache")
            return receipt
        lad_canonical = next(c.canonical for c in chain.components if c.label == "local_app_data")
        receipt["path_identity"] = {
            "package_family": chain.package_family,
            "forms": {c.label: c.form for c in chain.components if c.form != "exact"},
            "cache_base": _redact(chain.canonical, [lad_canonical]),
        }
        cache_forms = [chain.components[-1].requested, chain.canonical]
        phase = "consumer_guard"
        rows = processes()
        guard = classify_processes(rows, owned_roots=owned_roots, packaged_roots=cache_forms + packaged_roots,
                                   self_pid=self_pid, excluded_pids=excluded_pids)
        receipt["consumer_guard"] = guard
        by_pid = {r.pid: r for r in rows}
        in_use = set()
        for conflict in guard["conflicts"]:
            row = by_pid.get(conflict["pid"])
            key = _cache_key_of(row.executable, cache_forms) if row is not None and row.executable else None
            if key is not None:
                in_use.add(key)
        phase = "scan_cache"
        entries = scan(Path(chain.canonical))
        phase = "plan"
        receipt["cache_plan"] = plan_cache_entries(entries, requested_key=requested_key, in_use=in_use,
                                                   workbench_running=guard["workbench_app_running"])
        receipt["status"] = "refused" if guard["conflicts"] else "ready_for_owner_review"
        return receipt
    except PreflightRefused as refused:
        receipt.update(status="refused", phase=refused.phase or phase, refusal_code=refused.code)
        return receipt
    except OSError as error:
        receipt.update(status="refused", phase=phase, refusal_code="os_error",
                       os_error=type(error).__name__, winerror=getattr(error, "winerror", None))
        return receipt
    finally:
        probe.close()


RECEIPT_DIR = Path(__file__).resolve().parents[1] / ".local" / "mcp-cache-preflight"


def write_receipt(receipt: dict, directory: Path = RECEIPT_DIR) -> Path:
    """Write a new receipt file (never overwrite) and return its path. The
    receipt carries only redacted paths, keys, counts and fixed codes."""
    directory.mkdir(parents=True, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    target = directory / f"preflight-{stamp}-{os.urandom(4).hex()}.json"
    with target.open("x", encoding="utf-8", newline="\n") as handle:
        json.dump(receipt, handle, indent=2, sort_keys=True)
        handle.write("\n")
    return target


def main(argv: list[str] | None = None) -> int:  # pragma: no cover - CLI wrapper
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--requested-key", help="SHA-256 of the vw-mcp-server.sha256 manifest about to run")
    parser.add_argument("--owned-root-pid", type=int, action="append", default=[],
                        help="PID whose process tree this task owns (repeatable)")
    parser.add_argument("--packaged-root", action="append", default=[],
                        help="additional exact packaged image directory treated as the runtime")
    parser.add_argument("--exclude-pid", type=int, action="append", default=[],
                        help="PID the owner has reviewed and explicitly excludes from the guard (repeatable)")
    parser.add_argument("--local-app-data", help="inspect this app-data root instead of %%LOCALAPPDATA%% "
                        "(read-only; e.g. another package container's LocalCache\\Local)")
    args = parser.parse_args(argv)
    receipt: dict
    env_lad = os.environ.get("LOCALAPPDATA")
    if os.name != "nt":
        receipt = {"kind": "mcp-cache-preflight-dry-run", "status": "refused", "refusal_code": "windows_only"}
    elif not env_lad:
        receipt = {"kind": "mcp-cache-preflight-dry-run", "status": "refused",
                   "refusal_code": "local_app_data_unavailable"}
    else:
        lad = args.local_app_data or env_lad
        receipt = run(lad, args.requested_key, [os.getpid()] + args.owned_root_pid, args.packaged_root,
                      WindowsProbe(), windows_processes, self_pid=os.getpid(), excluded_pids=args.exclude_pid)
        receipt["local_app_data_source"] = "override" if args.local_app_data else "environment"
        if args.local_app_data:
            shown = _redact(args.local_app_data, [env_lad])
            receipt["local_app_data_override"] = shown
            identity = receipt.get("path_identity")
            if identity and identity["cache_base"].startswith("%LOCALAPPDATA%"):
                # run() redacts against the root it inspected; show it against
                # the environment's root so the receipt is not misleading.
                identity["cache_base"] = shown + identity["cache_base"][len("%LOCALAPPDATA%"):]
        # Evidence for the namespace question: does the environment's own
        # app-data child exist as seen by this process?
        receipt["environment_anchor_visible"] = os.path.isdir(os.path.join(env_lad, ANCHOR_NAME))
    try:
        path = write_receipt(receipt)
        receipt_path = path.relative_to(RECEIPT_DIR.parents[1]).as_posix()
    except OSError:
        receipt_path = None
    print(json.dumps(dict(receipt, receipt_path=receipt_path), indent=2, sort_keys=True))
    if receipt_path is None:
        return 1
    return 0 if receipt["status"] in ("ready_for_owner_review", "cache_absent_in_this_namespace") else 1


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
