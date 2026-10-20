#!/usr/bin/env python3
"""Bind explicit finalized-image MSI inputs and receipts; never run a process."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import importlib.util
import math
import os
from pathlib import Path
import re
import struct
import sys
import uuid
import zipfile

# Isolated Python (-I -S) omits the script directory from sys.path. Load only
# this repository-owned dependency by its fixed adjacent path, never a manifest.
_spec = importlib.util.spec_from_file_location("vw_msi_assemble", Path(__file__).with_name("assemble.py"))
if _spec is None or _spec.loader is None:
    raise RuntimeError("MSI validator unavailable")
a = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = a
_spec.loader.exec_module(a)

ROOT = Path(__file__).absolute().parents[2]
MAX_FILES = 24000
MAX_TOTAL = 8 * 1024**3
MAX_PACKAGE = 2 * 1024**3
METADATA = {"name": "VisualWorkbenchDev", "version": "0.0.1", "vendor": "Visual Workbench",
            "upgrade_uuid": "127902df-9261-5ead-8845-df9bc2cd90b4",
            "per_user_install": True, "start_menu": True}
PACKAGE = "VisualWorkbenchDev-0.0.1.msi"
BACKEND = ("tools/release/msi.py", "tools/release/assemble.py", "tools/release/build-msi.ps1",
           "tools/release/test_msi_wrapper.ps1",
           "tools/desktop-test/reports.py", "tools/desktop-test/personal_package.py",
           "tools/ffi-test/check_apk.py", "tools/process.psm1", "tools/desktop-test/lease.psm1")
Rejected = a.Rejected
require = a.require


def blob(raw: bytes) -> dict:
    return {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}


def read_json(inputs: a.Inputs, relative: str, sha256: str | None = None) -> tuple[dict, dict]:
    require(sha256 is None or isinstance(sha256, str) and a.HASH.fullmatch(sha256), "json_digest")
    actual, raw = inputs.read(relative, None, a.MAX_JSON, collect=True)
    require(sha256 is None or actual["sha256"] == sha256, "json_binding")
    return a.parse(raw), actual


def disjoint(left: Path, right: Path) -> bool:
    # casefold also refuses Windows aliases when the fixtures run on POSIX.
    left, right = str(left).rstrip("/\\").casefold(), str(right).rstrip("/\\").casefold()
    return left != right and not any(left.startswith(right + sep) or right.startswith(left + sep)
                                     for sep in ("/", "\\"))


def inventory(directory: Path, stack: ExitStack, cache: dict, *, limit: int = MAX_FILES) -> dict:
    """Observe a complete ordinary namespace through pinned ancestor handles."""
    source = a.Inputs(directory, stack, cache)
    pending = [("", a.get_dir(directory, stack, cache))]
    files, directories, total, count = {}, {}, 0, 1
    folded = set()
    while pending:
        relative, parent = pending.pop()
        before = parent.metadata()
        directories[relative] = list(a.identity(before))
        with parent.entries() as entries:
            for entry in entries:
                child = a.name(entry.name if not relative else relative + "/" + entry.name)
                count += 1
                require(count <= limit, "inventory_count")
                require(child.casefold() not in folded, "inventory_case_alias")
                folded.add(child.casefold())
                current = parent.stat(entry.name)
                if a.stat.S_ISDIR(current.st_mode):
                    a.ordinary(current, True)
                    pending.append((child, a.get_dir(parent.path / entry.name, stack, cache)))
                else:
                    a.ordinary(current, False)
                    actual, _ = source.read(child, None, a.MAX_FILE)
                    total += actual["bytes"]
                    require(total <= MAX_TOTAL, "inventory_total")
                    files[child] = actual | {"snapshot": list(source.seen[child])}
        require(a.snapshot(parent.metadata()) == a.snapshot(before), "inventory_changed")
        parent.check()
    return {"root": str(directory), "files": dict(sorted(files.items())),
            "directories": dict(sorted(directories.items()))}


def declarations(rows) -> dict:
    require(isinstance(rows, list) and 0 < len(rows) <= MAX_FILES, "tool_file_count")
    for row in rows:
        a.descriptor(row)
    a.unique_paths(rows)
    require(sum(row["bytes"] for row in rows) <= MAX_TOTAL, "tool_total")
    return {row["path"]: a.blob(row) for row in rows}


def executable_shape(source: a.Inputs, path: str, kind: str) -> None:
    directory = a.get_dir((source.root / path).parent, source.stack, source.cache)
    with directory.open(Path(path).name) as stream:
        header = stream.read(64)
        require(len(header) == 64 and header[:2] == b"MZ", "tool_pe")
        offset = struct.unpack_from("<I", header, 60)[0]
        require(64 <= offset <= 1024**2, "tool_pe")
        stream.seek(offset)
        pe = stream.read(6)
        require(len(pe) == 6 and pe[:4] == b"PE\0\0" and
                struct.unpack_from("<H", pe, 4)[0] in ((0x14c, 0x8664) if kind == "wix" else (0x8664,)), "tool_pe")


def python_layout(base: Path, version: str, declared: dict) -> None:
    for name in declared:
        require(Path(name).name.casefold() != "pyvenv.cfg" and not name.casefold().endswith("._pth"),
                "python_layout")
    minor = version.split(".")[1]
    require("python3" + minor + ".dll" in declared and
            ("Lib/os.py" in declared or "python3" + minor + ".zip" in declared) and
            not os.path.lexists(base.parent / "pyvenv.cfg"), "python_layout")


def toolchain(spec: dict, work: Path, stack: ExitStack, cache: dict) -> dict:
    require(set(spec) == {"schema", "jdk", "wix", "python", "evidence"} and
            spec["schema"] == "vw-msi-toolchain-v1", "toolchain_schema")
    require(isinstance(spec["evidence"], dict) and
            set(spec["evidence"]) == {"inventory", "compatibility", "license_review"}, "toolchain_evidence")
    for row in spec["evidence"].values():
        a.descriptor(row, a.MAX_JSON)
        require(row["bytes"] > 0, "toolchain_evidence")
    a.unique_paths(list(spec["evidence"].values()))
    result = {}
    for kind in ("jdk", "wix", "python"):
        value = spec[kind]
        require(isinstance(value, dict) and set(value) == {"root", "version", "roles", "files"}, "tool_schema")
        require(isinstance(value["root"], str) and isinstance(value["version"], str), "tool_schema")
        base = a.absolute(Path(value["root"]))
        require(";" not in str(base), "tool_path_separator")
        require(disjoint(base, work) and disjoint(base, ROOT / a.REPORTS.DIST), "tool_work_overlap")
        require(all(disjoint(base, Path(other["root"])) for other in result.values()), "tool_root_overlap")
        roles = value["roles"]
        require(isinstance(roles, dict), "tool_roles")
        if kind == "jdk":
            require(re.fullmatch(r"21\.[0-9A-Za-z.+_-]{1,80}", value["version"]) and
                    roles == {"jpackage": "bin/jpackage.exe", "java": "bin/java.exe"}, "jdk_profile")
        elif kind == "wix":
            require(re.fullmatch(r"3\.[0-9A-Za-z.+_-]{1,80}", value["version"]) and
                    set(roles) == {"candle", "light"}, "wix_profile")
            require(all(isinstance(path, str) for path in roles.values()), "tool_roles")
            require(Path(roles["candle"]).name == "candle.exe" and Path(roles["light"]).name == "light.exe" and
                    Path(roles["candle"]).parent == Path(roles["light"]).parent, "wix_roles")
        else:
            require(re.fullmatch(r"3\.(?:1[0-9]|[2-9][0-9])\.[0-9A-Za-z.+_-]{1,80}", value["version"]) and
                    roles == {"python": "python.exe"}, "python_profile")
        for path in roles.values():
            a.name(path)
            require(";" not in path, "tool_path_separator")
        declared = declarations(value["files"])
        if kind == "python":
            python_layout(base, value["version"], declared)
        require(set(roles.values()) <= declared.keys() and
                all(declared[path]["bytes"] > 0 for path in roles.values()), "tool_roles_missing")
        observed = inventory(base, stack, cache)
        require({name: a.blob(row) for name, row in observed["files"].items()} == declared,
                "tool_inventory_binding")
        tool_inputs = a.Inputs(base, stack, cache)
        for path in roles.values():
            executable_shape(tool_inputs, path, kind)
        require(inventory(base, stack, cache) == observed, "tool_inventory_changed")
        result[kind] = observed | {"version": value["version"], "roles": roles}
        require(sum(len(tool["files"]) for tool in result.values()) <= MAX_FILES and
                sum(row["bytes"] for tool in result.values() for row in tool["files"].values()) <= MAX_TOTAL,
                "tool_inventory_limits")
    if os.name == "nt":
        require(str(Path(sys.executable).absolute()).casefold() ==
                str(Path(result["python"]["root"]) / result["python"]["roles"]["python"]).casefold(),
                "python_runtime_binding")
    return result


def admit(root: Path, source: a.Inputs, catalog: dict | None) -> dict:
    require(root == ROOT, "tool_checkout_required")
    built = a.REPORTS.check(root)
    report, _ = a.PERSONAL.verify(root, None if catalog is None else root / catalog["path"],
                                None if catalog is None else catalog["sha256"])
    require(report.get("desktop_build_nonce") == built.get("nonce"), "personal_build_binding")
    closure = dict(built["source"])
    closure.update({"target/debug/" + name: row for name, row in built["distribution"]["native_inputs"].items()})
    closure.update({a.REPORTS.DIST + "/" + name: row for name, row in built["distribution"]["files"].items()})
    for name in (a.REPORTS.BEGIN, a.REPORTS.BUILT, "apps/android/build/outputs/apk/debug/android-debug.apk", *BACKEND):
        actual, _ = source.read(name, None, a.MAX_FILE)
        require(name not in closure or closure[name] == actual, "source_binding")
        closure[name] = actual
    if catalog is not None:
        closure[catalog["path"]] = a.blob(catalog)
    for name, expected in closure.items():
        source.read(name, expected, a.MAX_FILE)
    return {"desktop_nonce": built["nonce"], "desktop_report": blob(a.json_bytes(built)),
            "personal_package_report": blob(a.json_bytes(report)), "files": closure}


def inputs(root: Path, toolchain_name: str, toolchain_sha256: str, work: Path,
           catalog_name: str | None, catalog_sha256: str | None, stack: ExitStack, cache: dict) -> dict:
    source = a.Inputs(root, stack, cache)
    spec, actual = read_json(source, a.name(toolchain_name), toolchain_sha256)
    require((catalog_name is None) == (catalog_sha256 is None), "catalog_arguments")
    catalog = None
    if catalog_name is not None:
        require(isinstance(catalog_sha256, str) and a.HASH.fullmatch(catalog_sha256), "catalog_digest")
        record, _ = source.read(a.name(catalog_name), None, 32 * 1024)
        require(record["sha256"] == catalog_sha256 and record["bytes"] > 0, "catalog_binding")
        catalog = {"path": catalog_name, **record}
    tools = toolchain(spec, work, stack, cache)
    admission = admit(root, source, catalog)
    closure = admission.pop("files")
    closure[toolchain_name] = actual
    for row in spec["evidence"].values():
        source.read(row["path"], row, a.MAX_JSON)
        require(row["path"] not in closure or closure[row["path"]] == a.blob(row), "source_binding")
        closure[row["path"]] = a.blob(row)
    require(0 < len(closure) <= MAX_FILES and sum(row["bytes"] for row in closure.values()) <= MAX_TOTAL,
            "input_limits")
    a.unique_paths([{"path": name} for name in closure])
    for name, expected in closure.items():
        require(disjoint(root / name, work), "work_input_overlap")
        source.read(name, expected, a.MAX_FILE)
    image = inventory(root / a.REPORTS.DIST, stack, cache, limit=a.REPORTS.MAX_FILES)
    require({a.REPORTS.DIST + "/" + name: a.blob(row) for name, row in image["files"].items()} ==
            {name: row for name, row in closure.items() if name.startswith(a.REPORTS.DIST + "/")}, "image_binding")
    return {"admission": admission, "toolchain": {"path": toolchain_name, **actual}, "catalog": catalog,
            "input_bindings": {name: row | {"snapshot": list(source.seen[name])}
                               for name, row in sorted(closure.items())}, "tool_bindings": tools,
            "image_binding": image}


def invocation(bound: dict, root: Path, work: Path) -> dict:
    jdk, wix = bound["tool_bindings"]["jdk"], bound["tool_bindings"]["wix"]
    java_bin = Path(jdk["root"]) / "bin"
    wix_bin = (Path(wix["root"]) / wix["roles"]["candle"]).parent
    return {"executable": str(Path(jdk["root"]) / jdk["roles"]["jpackage"]),
            "argv": ["--type", "msi", "--app-image", str(root / a.REPORTS.DIST),
                     "--dest", str(work / "dest"), "--temp", str(work / "temp"),
                     "--name", METADATA["name"], "--app-version", METADATA["version"],
                     "--vendor", METADATA["vendor"], "--win-upgrade-uuid", METADATA["upgrade_uuid"],
                     "--win-per-user-install", "--win-menu"],
            "working_dir": str(work),
            "minimal_env": {"PATH": str(wix_bin) + ";" + str(java_bin), "JAVA_HOME": jdk["root"],
                            "TEMP": str(work / "runtime-temp"), "TMP": str(work / "runtime-temp")}}


def lease_inputs(bound: dict, root: Path, work: Path) -> list:
    files = {str(root / name) for name in bound["input_bindings"]}
    for tool in bound["tool_bindings"].values():
        files.update(str(Path(tool["root"]) / name) for name in tool["files"])
    files.add(str(work / "plan.json"))
    directories = {str(work), *(str(work / name) for name in ("dest", "temp", "runtime-temp"))}
    for observed in (*bound["tool_bindings"].values(), bound["image_binding"]):
        directories.update(str(Path(observed["root"]) / name) for name in observed["directories"])
    # The wrapper also pins every ancestor of these exact known leaves/dirs.
    return ([{"path": path, "directory": True} for path in sorted(directories)] +
            [{"path": path, "directory": False} for path in sorted(files)])


def write_json(directory: a.Directory, name: str, value: dict) -> dict:
    raw = a.json_bytes(value)
    with directory.open(name, True) as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
        own = a.snapshot(a.fstat_metadata(stream.fileno()))
    require(a.snapshot(directory.stat(name)) == own, "output_changed")
    directory.check()
    return blob(raw)


def validate_scope(root: Path, work: Path, timeout: int) -> tuple[Path, Path]:
    root, work = a.absolute(root), a.absolute(work)
    require(root == ROOT and type(timeout) is int and 1 <= timeout <= 3600, "scope")
    require(disjoint(work, root), "work_checkout_overlap")
    return root, work


def plan(root: Path, toolchain_name: str, toolchain_sha256: str, work: Path, timeout: int,
         catalog_name: str | None = None, catalog_sha256: str | None = None) -> dict:
    root, work = validate_scope(root, work, timeout)
    with ExitStack() as stack:
        cache = {}
        parent = a.get_dir(work.parent, stack, cache)
        require(not os.path.lexists(work), "fresh_work_required")
        bound = inputs(root, toolchain_name, toolchain_sha256, work, catalog_name, catalog_sha256, stack, cache)
        parent.mkdir(work.name)
        directory = a.get_dir(work, stack, cache)
        directory.mkdir("dest")
        directory.mkdir("temp")
        directory.mkdir("runtime-temp")
        identities = {name: list(a.get_dir(work / name, stack, cache).ident)
                      for name in ("dest", "temp", "runtime-temp")}
        value = {"schema": "vw-msi-plan-v1", "run_nonce": uuid.uuid4().hex, "root": str(root), "work": str(work),
                 "metadata": METADATA, "timeout_seconds": timeout, **bound, **invocation(bound, root, work),
                 "work_identity": {"root": list(directory.ident), **identities},
                 "expected_package": PACKAGE, "lease_inputs": lease_inputs(bound, root, work)}
        write_json(directory, "plan.json", value)
        return value


def verify_plan(root: Path, plan_path: Path, plan_sha256: str, stack: ExitStack, cache: dict,
                *, completed: bool = False) -> tuple[dict, dict, a.Directory]:
    plan_path = a.absolute(plan_path)
    require(plan_path.name == "plan.json", "plan_path")
    work = plan_path.parent
    source = a.Inputs(work, stack, cache)
    value, plan_blob = read_json(source, "plan.json", plan_sha256)
    required = {"schema", "run_nonce", "root", "work", "metadata", "timeout_seconds", "admission", "toolchain",
                "catalog", "input_bindings", "tool_bindings", "executable", "argv", "working_dir", "minimal_env",
                "work_identity", "expected_package", "lease_inputs", "image_binding"}
    require(set(value) == required and value["schema"] == "vw-msi-plan-v1" and
            isinstance(value["run_nonce"], str) and re.fullmatch(r"[0-9a-f]{32}", value["run_nonce"]), "plan_schema")
    timeout = value["timeout_seconds"]
    root, work = validate_scope(root, work, timeout)
    require(value["root"] == str(root) and value["work"] == str(work) and value["metadata"] == METADATA and
            value["timeout_seconds"] == timeout and value["expected_package"] == PACKAGE, "plan_scope")
    directory = a.get_dir(work, stack, cache)
    require(isinstance(value["work_identity"], dict) and
            set(value["work_identity"]) == {"root", "dest", "temp", "runtime-temp"},
            "work_identity")
    require(value["work_identity"]["root"] == list(directory.ident), "work_identity")
    for name in ("dest", "temp", "runtime-temp"):
        require(value["work_identity"][name] == list(a.get_dir(work / name, stack, cache).ident), "work_identity")
    expected_names = ({"plan.json", "dest", "temp", "runtime-temp", "process.json"} if completed else
                      {"plan.json", "dest", "temp", "runtime-temp"})
    observed_names = set()
    with directory.entries() as entries:
        for entry in entries:
            require(entry.name in expected_names and entry.name not in observed_names, "work_namespace")
            observed_names.add(entry.name)
    require(observed_names == expected_names, "work_namespace")
    manifest = a.descriptor(value["toolchain"], a.MAX_JSON)
    catalog = value["catalog"]
    if catalog is not None:
        a.descriptor(catalog, 32 * 1024)
    bound = inputs(root, manifest["path"], manifest["sha256"], work,
                   None if catalog is None else catalog["path"], None if catalog is None else catalog["sha256"],
                   stack, cache)
    require(all(value[key] == item for key, item in bound.items()), "plan_input_drift")
    require(all(value[key] == item for key, item in invocation(bound, root, work).items()), "plan_invocation")
    require(value["lease_inputs"] == lease_inputs(bound, root, work), "plan_leases")
    if not completed:
        for name in ("dest", "temp", "runtime-temp"):
            with a.get_dir(work / name, stack, cache).entries() as entries:
                require(next(entries, None) is None, "work_not_empty")
    return value, plan_blob, directory


def check(root: Path, plan_path: Path, plan_sha256: str) -> dict:
    with ExitStack() as stack:
        value, actual, _ = verify_plan(root, plan_path, plan_sha256, stack, {})
        return {"schema": "vw-msi-preflight-v1", "run_nonce": value["run_nonce"], "plan": actual,
                "status": "observed-inputs-match-plan"}


def process(value: dict, planned: dict, plan_sha256: str) -> dict:
    keys = {"schema", "run_nonce", "plan_sha256", "executable", "argv", "working_dir", "minimal_env", "os_env", "runner"}
    require(set(value) == keys and value["schema"] == "vw-msi-process-v1" and
            value["run_nonce"] == planned["run_nonce"] and value["plan_sha256"] == plan_sha256 and
            all(value[key] == planned[key] for key in ("executable", "argv", "working_dir", "minimal_env")),
            "process_binding")
    os_environment(value["os_env"], planned["minimal_env"])
    proof = value["runner"]
    require(isinstance(proof, dict) and proof.get("phase") == "desktop-msi" and
            type(proof.get("exit_code")) is int and proof["exit_code"] == 0 and
            type(proof.get("parent_exit_code")) is int and proof["parent_exit_code"] == 0 and
            proof.get("failure_reason") is None and proof.get("contained_in_windows_job") is True and
            proof.get("process_tree_cleanup_confirmed") is True and proof.get("output_streams_completed") is True and
            proof.get("surviving_processes_before_cleanup") == [] and
            proof.get("compiler_telemetry_cleaned") is False and
            type(proof.get("timeout_seconds")) is int and proof["timeout_seconds"] == planned["timeout_seconds"],
            "process_failed")
    elapsed = proof.get("elapsed_seconds")
    require(type(elapsed) in (int, float) and math.isfinite(elapsed) and 0 < elapsed <= planned["timeout_seconds"] + 65,
            "process_timing")
    return value


def windows_directory() -> Path:
    import ctypes
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.GetWindowsDirectoryW.argtypes = (ctypes.c_wchar_p, ctypes.c_uint)
    api.GetWindowsDirectoryW.restype = ctypes.c_uint
    buffer = ctypes.create_unicode_buffer(32768)
    count = api.GetWindowsDirectoryW(buffer, len(buffer))
    require(0 < count < len(buffer), "os_environment")
    return a.absolute(Path(buffer.value))


def os_environment(value: dict, minimal: dict) -> None:
    require(isinstance(value, dict) and set(value) == {"SystemRoot", "WINDIR", "ComSpec", "PATH"} and
            all(isinstance(item, str) for item in value.values()), "os_environment")
    windows = a.absolute(Path(value["SystemRoot"]))
    require(value["WINDIR"] == str(windows) and value["ComSpec"] == str(windows / "System32/cmd.exe") and
            value["PATH"] == minimal["PATH"] + ";" + str(windows / "System32"), "os_environment")
    if os.name == "nt":
        require(str(windows).rstrip("\\").casefold() == str(windows_directory()).rstrip("\\").casefold(),
                "os_environment")
        a.REPORTS.plain(windows, True)
        a.REPORTS.plain(windows / "System32/cmd.exe")


def cfb_shape(raw: bytes, size: int) -> dict:
    require(len(raw) == 512 and raw[:8] == bytes.fromhex("d0cf11e0a1b11ae1") and raw[8:24] == bytes(16), "msi_cfb_header")
    minor, major, order, shift, mini = struct.unpack_from("<5H", raw, 24)
    require(minor == 0x3e and (major, shift) in ((3, 9), (4, 12)) and order == 0xfffe and mini == 6 and
            raw[34:40] == bytes(6), "msi_cfb_profile")
    sector = 1 << shift
    require(sector * 3 <= size <= MAX_PACKAGE and size % sector == 0, "msi_cfb_size")
    count = size // sector - 1
    directories, fats, first_dir, transaction, cutoff, first_mini, mini_count, first_difat, difat_count = struct.unpack_from("<9I", raw, 40)
    end = 0xfffffffe
    require((major != 3 or directories == 0) and (major != 4 or 1 <= directories <= count) and
            1 <= fats <= count and first_dir < count and transaction == 0 and cutoff == 4096 and
            ((mini_count == 0 and first_mini == end) or (1 <= mini_count <= count and first_mini < count)) and
            ((difat_count == 0 and first_difat == end) or (1 <= difat_count <= count and first_difat < count)),
            "msi_cfb_layout")
    difat = struct.unpack_from("<109I", raw, 76)
    used = min(fats, 109)
    require(all(item < count for item in difat[:used]) and len(set(difat[:used])) == used and
            all(item == 0xffffffff for item in difat[used:]) and (fats <= 109 or difat_count > 0), "msi_cfb_difat")
    return {"format": "OLE-CFB", "major_version": major, "sector_bytes": sector,
            "header_and_sector_layout_checked": True, "msi_database_validity_verified": False}


def record(root: Path, plan_path: Path, plan_sha256: str, process_path: Path) -> dict:
    work = a.absolute(plan_path).parent
    require(a.absolute(process_path) == work / "process.json", "process_path")
    with ExitStack() as stack:
        cache = {}
        value, plan_blob, directory = verify_plan(root, plan_path, plan_sha256, stack, cache, completed=True)
        source = a.Inputs(work, stack, cache)
        envelope, envelope_blob = read_json(source, "process.json")
        process(envelope, value, plan_blob["sha256"])
        output = inventory(work / "dest", stack, cache)
        require(set(output["files"]) == {PACKAGE} and set(output["directories"]) == {""}, "msi_output_namespace")
        package = output["files"][PACKAGE]
        require(0 < package["bytes"] <= MAX_PACKAGE, "msi_size")
        destination = a.Inputs(work / "dest", stack, cache)
        parent = a.get_dir(work / "dest", stack, cache)
        with parent.open(PACKAGE) as stream:
            require(list(a.snapshot(a.fstat_metadata(stream.fileno()))) == package["snapshot"], "msi_changed")
            shape = cfb_shape(stream.read(512), package["bytes"])
        destination.read(PACKAGE, a.blob(package), MAX_PACKAGE)
        temporary = inventory(work / "temp", stack, cache)
        runtime_temporary = inventory(work / "runtime-temp", stack, cache)
        # Recheck after output reads. Separate invocations are observed snapshots;
        # the wrapper holds known input leases through preflight/process/record.
        verify_plan(root, plan_path, plan_sha256, stack, cache, completed=True)
        require(inventory(work / "dest", stack, cache) == output and
                inventory(work / "temp", stack, cache) == temporary and
                inventory(work / "runtime-temp", stack, cache) == runtime_temporary, "output_changed")
        source.read("process.json", envelope_blob, a.MAX_JSON)
        result = {"schema": "vw-msi-receipt-v1", "status": "observed-msi-output-hash-bound",
                  "run_nonce": value["run_nonce"], "plan": plan_blob, "process": envelope_blob,
                  "metadata": METADATA, "package": {"path": "dest/" + PACKAGE, **a.blob(package), "shape": shape},
                  "admission": value["admission"], "toolchain": value["toolchain"],
                  "input_binding": "exact observed inputs and supplied root-admitted tool inventory; known-file leases belong to the wrapper",
                  "temporary_inventory": temporary, "runtime_temporary_inventory": runtime_temporary,
                  "atomic_namespace_verified": False,
                  "payload_verified": False, "msi_database_validity_verified": False,
                  "signature_verified": False, "installed_or_launched": False, "product_acceptance": False}
        write_json(directory, "receipt.json", result)
        return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("plan")
    prepare.add_argument("--root", type=Path, required=True)
    prepare.add_argument("--toolchain", required=True)
    prepare.add_argument("--toolchain-sha256", required=True)
    prepare.add_argument("--work", type=Path, required=True)
    prepare.add_argument("--timeout-seconds", type=int, required=True)
    prepare.add_argument("--catalog")
    prepare.add_argument("--catalog-sha256")
    for command in ("check", "record"):
        existing = commands.add_parser(command)
        existing.add_argument("--root", type=Path, required=True)
        existing.add_argument("--plan", type=Path, required=True)
        existing.add_argument("--plan-sha256", required=True)
        if command == "record":
            existing.add_argument("--process", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "plan":
            plan(args.root, args.toolchain, args.toolchain_sha256, args.work, args.timeout_seconds,
                 args.catalog, args.catalog_sha256)
        elif args.command == "check":
            check(args.root, args.plan, args.plan_sha256)
        else:
            record(args.root, args.plan, args.plan_sha256, args.process)
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError, struct.error, zipfile.BadZipFile) as error:
        code = str(error) if isinstance(error, Rejected) else "input_or_validator_refused"
        print("MSI_REJECTED:" + code)
        return 1
    print("MSI_OK:" + args.command)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
