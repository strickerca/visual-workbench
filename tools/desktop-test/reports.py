"""Source/distribution binding and strict startup receipts; never launches a process."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import sys
import time
import uuid
import zipfile

APP = "VisualWorkbenchDev"
DIST = "apps/desktop/build/compose/binaries/main/app/" + APP
EXE = APP + ".exe"
MAX_FILES = 4096
MAX_TOTAL = 4 * 1024**3
MAX_FILE = 1024**3
MAX_JSON = 8 * 1024**2
NATIVES = ("vw-connection-helper.exe", "vw_core.dll", "vw_host.dll")
INPUTS = (*NATIVES, "vw-bindgen.exe")
BEGIN = ".local/desktop-test-build-start.json"
BUILT = ".local/desktop-test-build.json"
NONCE_RESOURCE = "vw-desktop-build.id"
HASH = re.compile(r"[0-9a-f]{64}\Z")
NONCE = re.compile(r"[0-9a-f]{32}\Z")
READY = re.compile(r"VW_DESKTOP_READY startup_ms=([0-9]{1,8}) composeDensity=([0-9]+(?:\.[0-9]+)?) pmv2=true\Z")
FRAME = re.compile(r"VW_DESKTOP_SMOKE_FRAME window_dpi=([0-9]{1,4}) window_pmv2=true drawn=true\Z")
EXIT = "VW_DESKTOP_EXIT startup_smoke=true cleanup=complete"


class Rejected(ValueError):
    """Only static error codes cross the runner boundary."""


def require(ok: bool, code: str) -> None:
    if not ok:
        raise Rejected(code)


def plain(path: Path, directory: bool = False) -> Path:
    path = path.absolute()
    for component in (path, *path.parents):
        info = component.lstat()
        require(not stat.S_ISLNK(info.st_mode) and not getattr(info, "st_file_attributes", 0) & 0x400,
                "redirected_path")
    require(path.is_dir() if directory else path.is_file(), "path_type")
    return path


def relative_name(name: str) -> None:
    require(isinstance(name, str) and 1 <= len(name) <= 512 and "\\" not in name and ":" not in name,
            "relative_name")
    require(not name.startswith("/") and all(p not in ("", ".", "..") and not p.endswith((" ", "."))
            for p in name.split("/")), "relative_name")
    require(all(32 <= ord(c) < 127 for c in name) and not re.search(r'[<>"|?*]', name), "relative_name")
    require(len(PurePosixPath(name).parts) <= 24, "path_depth")
    for component in name.split("/"):
        require(not re.fullmatch(r"(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?", component, re.I),
                "relative_name")


def files(root: Path, *, source: bool = False) -> list[Path]:
    plain(root, True)
    found = []
    count = 0
    for parent, directories, names in os.walk(root, followlinks=False):
        if source:
            directories[:] = [x for x in directories if x not in
                              ("build", "build-hil", "target", "__pycache__", ".gradle", ".git", "node_modules")]
        count += len(directories) + len(names)
        require(count <= (24000 if source else MAX_FILES), "inventory_count")
        for name in directories:
            plain(Path(parent) / name, True)
        for name in names:
            path = Path(parent) / name
            relative_name(path.relative_to(root).as_posix())
            plain(path)
            if not source or path.suffix not in (".pyc", ".log"):
                found.append(path)
    return sorted(found)


def digest(path: Path, limit: int = MAX_FILE) -> dict:
    plain(path)
    before = path.stat()
    require(0 <= before.st_size <= limit, "file_size")
    sha = hashlib.sha256()
    count = 0
    with path.open("rb") as stream:
        while chunk := stream.read(1024**2):
            count += len(chunk)
            require(count <= limit, "file_size")
            sha.update(chunk)
    after = path.stat()
    require(count == before.st_size == after.st_size and before.st_mtime_ns == after.st_mtime_ns,
            "file_changed")
    return {"bytes": count, "sha256": sha.hexdigest()}


def bounded_json(path: Path) -> dict:
    plain(path)
    require(path.stat().st_size <= MAX_JSON, "json_size")
    def unique(pairs):
        out = {}
        for key, value in pairs:
            require(key not in out, "json_duplicate")
            out[key] = value
        return out
    with path.open("rb") as stream:
        raw = stream.read(MAX_JSON + 1)
    require(len(raw) <= MAX_JSON, "json_size")
    value = json.loads(raw.decode("utf-8-sig"), object_pairs_hook=unique)
    require(isinstance(value, dict), "json_object")
    return value


def save(path: Path, value: dict, *, replace: bool = False) -> None:
    plain(path.parent, True)
    raw = (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")
    require(len(raw) <= MAX_JSON, "json_size")
    if not replace:
        with path.open("xb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        return
    # Only fixed local build receipts use replacement; retain each run separately.
    if path.exists():
        plain(path)
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex)
    try:
        with temporary.open("xb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def environment_ok(environment: dict | None = None) -> None:
    environment = os.environ if environment is None else environment
    for name in ("JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH",
                 "JNA_LIBRARY_PATH", "SKIKO_LIBRARY_PATH"):
        require(not environment.get(name), "runtime_override")
    for name in ("vwNativeDir", "vwDesktopSmokeConsole", "vwDesktopBuildId"):
        require(not environment.get("ORG_GRADLE_PROJECT_" + name), "gradle_override")


def sources(root: Path) -> dict:
    required = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml", ".cargo/config.toml",
                "build.ps1", "tools/process.psm1", "tools/check_setup.py", "tools/enter-dev.ps1",
                "apps/gradlew.bat", "apps/build.gradle.kts", "apps/settings.gradle.kts", "apps/gradle.properties")
    selected = {root / p for p in required}
    for tree in ("core/crates", "host-win/crates", "contracts", "apps/desktop/src", "apps/shared/src",
                 "apps/gradle", "apps/bindings-core", "apps/bindings-host", "tools/desktop-test",
                 "tools/ffi-test", "third_party/notices"):
        selected.update(files(root / tree, source=True))
    # License and dependency graph inputs include every configured app module.
    for path in files(root / "apps", source=True):
        if path.name in ("build.gradle.kts", "gradle.lockfile", "gradle.properties"):
            selected.add(path)
    require(len(selected) <= 24000, "source_count")
    return {p.relative_to(root).as_posix(): digest(p) for p in sorted(selected)}


def jar_admission(path: Path) -> None:
    # Bound central-directory allocation BEFORE zipfile reads it. These small
    # app-image JARs do not need ZIP64 or multidisk support.
    with path.open("rb") as stream:
        size = path.stat().st_size
        require(22 <= size <= MAX_FILE, "jar_size")
        stream.seek(max(0, size - 65557))
        tail = stream.read(65557)
    position = tail.rfind(b"PK\x05\x06")
    require(position >= 0 and len(tail) - position >= 22, "jar_eocd")
    _, disk, central_disk, local_count, count, central_size, central_at, comment = struct.unpack_from("<4s4H2IH", tail, position)
    require(disk == central_disk == 0 and local_count == count and 0 < count < 65535 and
            central_size <= 16 * 1024**2 and central_at + central_size == size - len(tail) + position and
            position + 22 + comment == len(tail), "jar_directory")


def packaged_resources(directory: Path, native: dict, nonce: str) -> None:
    expected = {"win32-x86-64/" + name: native[name] for name in NATIVES}
    markers = {NONCE_RESOURCE, "vw-native-runtime.sha256", "com/visualworkbench/desktop/MainKt.class"}
    seen = set()
    jars = sorted((directory / "app").glob("*.jar"))
    require(1 <= len(jars) <= 256, "jar_count")
    for jar in jars:
        plain(jar)
        jar_admission(jar)
        with zipfile.ZipFile(jar) as archive:
            names = set()
            for entry in archive.infolist():
                require(entry.filename not in names, "jar_duplicate")
                names.add(entry.filename)
                if entry.filename not in expected and entry.filename not in markers:
                    continue
                require(entry.filename not in seen and not entry.flag_bits & 1, "resource_duplicate")
                seen.add(entry.filename)
                limit = expected.get(entry.filename, {}).get("bytes", 4096)
                if entry.filename.endswith("MainKt.class"):
                    limit = 4 * 1024**2
                require(0 < entry.file_size <= limit, "resource_size")
                sha = hashlib.sha256()
                count = 0
                body = bytearray()
                with archive.open(entry) as stream:
                    while chunk := stream.read(65536):
                        count += len(chunk)
                        require(count <= limit, "resource_size")
                        sha.update(chunk)
                        if entry.filename in (NONCE_RESOURCE, "vw-native-runtime.sha256"):
                            body.extend(chunk)
                require(count == entry.file_size, "resource_size")
                if entry.filename in expected:
                    require({"bytes": count, "sha256": sha.hexdigest()} == expected[entry.filename], "native_resource_binding")
                elif entry.filename == NONCE_RESOURCE:
                    require(body == (nonce + "\n").encode("ascii"), "stale_distribution")
                elif entry.filename == "vw-native-runtime.sha256":
                    wanted = "".join(f"{native[n]['sha256']} {native[n]['bytes']} {n}\n" for n in sorted(NATIVES))
                    require(body == wanted.encode("ascii"), "native_manifest_binding")
    require(seen == set(expected) | markers, "packaged_resources_missing")


def launcher_config(directory: Path, inventory: dict) -> None:
    config = directory / "app" / (APP + ".cfg")
    require(config.stat().st_size <= 65536, "launcher_config_size")
    text = config.read_text(encoding="utf-8-sig")
    section = None
    main = []
    classpath = []
    options = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line in ("[Application]", "[JavaOptions]", "[ArgOptions]"):
            section = line
            continue
        require("=" in line, "launcher_config")
        key, value = line.split("=", 1)
        if section == "[Application]" and key == "app.mainclass":
            main.append(value)
        elif section == "[Application]" and key == "app.classpath":
            for item in value.split(";"):
                require(item.startswith("$APPDIR\\") or item.startswith("$APPDIR/"), "external_classpath")
                name = item[8:]
                require("/" not in name and "\\" not in name and name.endswith(".jar"), "external_classpath")
                relative_name(name)
                require("app/" + name in inventory, "missing_classpath")
                classpath.append(name)
        elif section == "[JavaOptions]" and key == "java-options":
            options.append(value)
        else:
            raise Rejected("launcher_config")
    require(main == ["com.visualworkbench.desktop.MainKt"] and classpath and
            len(classpath) == len(set(classpath)), "launcher_main")
    require(set(classpath) == {Path(p).name for p in inventory if p.startswith("app/") and p.endswith(".jar")},
            "unlisted_classpath")
    required = {"-Djpackage.app-version=0.0.1", "-Dsun.java2d.dpiaware=true", "-Dskiko.renderApi=DIRECT3D"}
    # Compose's packager adds only confined resource/Skiko paths and its Swing
    # setup switch. Accept the literal Windows escaping forms, never an external
    # path, generic JVM switch, injected agent or duplicate property override.
    optional = {"-Dskiko.library.path=$APPDIR", "-Dcompose.application.configure.swing.globals=true"}
    optional.update("-Dcompose.application.resources.dir=$APPDIR" + sep + "resources"
                    for sep in ("/", "\\", "\\\\"))
    require(required <= set(options) <= required | optional and
            len({value.split("=", 1)[0] for value in options}) == len(options), "launcher_options")


def distribution(root: Path, nonce: str) -> dict:
    require(NONCE.fullmatch(nonce) is not None, "build_nonce")
    directory = plain(root / DIST, True)
    inventory = {}
    total = 0
    for path in files(directory):
        name = path.relative_to(directory).as_posix()
        require(name.casefold() not in {n.casefold() for n in inventory}, "distribution_case_collision")
        record = digest(path)
        total += record["bytes"]
        require(total <= MAX_TOTAL, "distribution_total")
        inventory[name] = record
    require({EXE, "app/" + APP + ".cfg", "runtime/bin/server/jvm.dll", "runtime/lib/modules"} <= set(inventory),
            "distribution_layout")
    # PE console subsystem is deliberate for redirected markers. Default app
    # builds retain their GUI launcher; the receipt records this packaging option.
    with (directory / EXE).open("rb") as stream:
        header = stream.read(64)
        require(len(header) == 64 and header[:2] == b"MZ", "launcher_pe")
        offset = struct.unpack_from("<I", header, 60)[0]
        require(64 <= offset <= 1024**2, "launcher_pe")
        stream.seek(offset)
        header = stream.read(96)
        require(len(header) == 96 and header[:4] == b"PE\0\0" and struct.unpack_from("<H", header, 4)[0] == 0x8664 and
                struct.unpack_from("<H", header, 24)[0] == 0x20b and struct.unpack_from("<H", header, 92)[0] == 3,
                "launcher_console")
    native = {name: digest(root / "target/debug" / name, 256 * 1024**2) for name in INPUTS}
    packaged_resources(directory, native, nonce)
    launcher_config(directory, inventory)
    return {"relative_directory": DIST, "launcher": EXE, "files": inventory, "native_inputs": native,
            "total_bytes": total, "console_launcher": True}


def begin(root: Path) -> dict:
    environment_ok()
    return {"schema": 1, "kind": "desktop-build-start", "nonce": uuid.uuid4().hex,
            "started_ns": time.time_ns(), "source": sources(root)}


def record(root: Path, started: dict) -> dict:
    environment_ok()
    require(started.get("schema") == 1 and started.get("kind") == "desktop-build-start", "build_start")
    require(started["source"] == sources(root), "source_drift")
    return {"schema": 1, "kind": "desktop-distribution", "nonce": started["nonce"],
            "started_ns": started["started_ns"], "recorded_ns": time.time_ns(),
            "source": started["source"], "distribution": distribution(root, started["nonce"]),
            "build_target": "build-desktop-distribution", "task": ":desktop:createDistributable"}


def check(root: Path) -> dict:
    environment_ok()
    built = bounded_json(root / BUILT)
    started = bounded_json(root / BEGIN)
    require(built.get("schema") == 1 and built.get("kind") == "desktop-distribution" and
            built.get("nonce") == started.get("nonce") and built.get("started_ns") == started.get("started_ns"), "build_receipt")
    require(built.get("source") == started.get("source") == sources(root), "source_drift")
    require(built.get("distribution") == distribution(root, built["nonce"]), "distribution_drift")
    return built


def startup(text: str) -> dict:
    require(len(text.encode("utf-8")) <= 1024**2 and "\0" not in text, "startup_output_size")
    markers = [line for line in text.splitlines() if "VW_DESKTOP_" in line]
    require(len(markers) == 3, "startup_marker_count")
    ready = READY.fullmatch(markers[0])
    frame = FRAME.fullmatch(markers[1])
    require(ready is not None and frame is not None and markers[2] == EXIT, "startup_marker_order")
    elapsed = int(ready[1])
    density = float(ready[2])
    dpi = int(frame[1])
    require(0 < elapsed <= 300000 and math.isfinite(density) and 0.25 <= density <= 16 and 48 <= dpi <= 1536,
            "startup_values")
    return {"main_to_drawn_frame_ms": elapsed, "compose_density": density, "window_dpi": dpi,
            "process_pmv2": True, "window_pmv2": True, "compose_draw_returned": True,
            "ordinary_quit_cleanup": True, "cold_start_or_performance_acceptance": False,
            "visual_acceptance": False}


def process_receipt(value: dict) -> dict:
    require(value.get("capture_byte_limit") == 1024**2 and value.get("capture_overflow") is False and
            type(value.get("captured_bytes")) is int and 0 <= value["captured_bytes"] < 1024**2 and
            value.get("raw_output_retained") is True, "startup_capture")
    require(value.get("phase") == "desktop-startup" and value.get("exit_code") == 0 and
            value.get("parent_exit_code") == 0 and value.get("failure_reason") is None and
            value.get("contained_in_windows_job") is True and value.get("process_tree_cleanup_confirmed") is True and
            value.get("output_streams_completed") is True and not value.get("surviving_processes_before_cleanup") and
            value.get("compiler_telemetry_cleaned") is False, "startup_process")
    elapsed = value.get("elapsed_seconds")
    require(type(elapsed) in (int, float) and math.isfinite(elapsed) and 0 < elapsed <= 330, "process_elapsed")
    return {key: value[key] for key in ("exit_code", "elapsed_seconds", "contained_in_windows_job",
                                      "process_tree_cleanup_confirmed", "output_streams_completed",
                                      "capture_byte_limit", "captured_bytes", "capture_overflow")}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("begin", "record", "check", "collect"))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--log", type=Path)
    parser.add_argument("--process", type=Path)
    args = parser.parse_args()
    try:
        root = plain(args.root, True)
        if args.command == "begin":
            local = root / ".local"
            local.mkdir(exist_ok=True)
            plain(local, True)
            save(root / BEGIN, begin(root), replace=True)
        elif args.command == "record":
            save(root / BUILT, record(root, bounded_json(root / BEGIN)), replace=True)
        elif args.command == "check":
            value = check(root)
            require(args.output is not None, "output_required")
            save(args.output, value)
        else:
            require(args.output is not None and args.log is not None and args.process is not None, "collect_arguments")
            built = check(root)
            plain(args.log)
            require(args.log.stat().st_size <= 1024**2, "startup_output_size")
            observation = startup(args.log.read_text(encoding="utf-8-sig"))
            run = process_receipt(bounded_json(args.process))
            save(args.output, {"schema": 1, "status": "passed", "kind": "desktop-startup", "build": built,
                              "observation": observation, "process": run, "ordinary_local_initialization": True,
                              "installed_or_signed": False, "firewall_changed": False, "screenshots_created": 0})
        print("DESKTOP_REPORT_OK:" + args.command)
        return 0
    except (Rejected, OSError, ValueError, KeyError, TypeError, OverflowError, zipfile.BadZipFile) as error:
        code = str(error) if isinstance(error, Rejected) else "malformed_or_unavailable_input"
        print("DESKTOP_REPORT_REJECTED:" + code)
        return 1


if __name__ == "__main__":
    sys.exit(main())
