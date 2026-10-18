"""Bound final personal package resources; no install, extraction or native execution."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import struct
import zipfile
import reports as r

CATALOG = "vw-remote-editor-catalog.json"
CATALOG_HASH = "vw-remote-editor-catalog.sha256"
MAX_NOTICE = 4 * 1024**2
MAX_NOTICE_TOTAL = 32 * 1024**2


def blob(value: bytes) -> dict:
    return {"bytes": len(value), "sha256": hashlib.sha256(value).hexdigest()}


def notices(root: Path) -> tuple[dict, list]:
    manifest = r.bounded_json(root / "third_party/LICENSES")
    r.require(manifest.get("schema_version") == 1 and isinstance(manifest.get("files"), list), "notice_manifest")
    r.require(0 < len(manifest["files"]) <= 512, "notice_count")
    found, records, total = {}, [], 0
    for row in manifest["files"]:
        r.require(isinstance(row, dict), "notice_record")
        name = row.get("path", "")
        r.relative_name(name)
        r.require(name.startswith("notices/") and len(name.split("/")) == 2, "notice_path")
        resource = name.removeprefix("notices/")
        r.require(resource.casefold() not in {n.casefold() for n in found}, "notice_duplicate")
        r.require(isinstance(row.get("license"), str) and 0 < len(row["license"]) <= 256 and
                  isinstance(row.get("source_url"), str) and 0 < len(row["source_url"]) <= 2048,
                  "notice_metadata")
        r.require(all(32 <= ord(ch) < 127 for ch in row["license"] + row["source_url"]), "notice_metadata")
        actual = r.digest(root / "third_party" / name, MAX_NOTICE)
        r.require(actual["bytes"] > 0 and actual["sha256"] == row.get("sha256"), "notice_source_binding")
        total += actual["bytes"]
        r.require(total <= MAX_NOTICE_TOTAL, "notice_total")
        found[resource] = actual
        records.append({"resource": resource, "license": row["license"], "source_url": row["source_url"], **actual})
    actual_names = {p.relative_to(root / "third_party/notices").as_posix() for p in r.files(root / "third_party/notices")}
    r.require(actual_names == set(found), "notice_unlisted_source")
    return found, records


def catalog(path: Path | None, expected: str | None) -> dict:
    r.require((path is None) == (expected is None), "catalog_arguments")
    if path is None:
        return {}
    r.require(isinstance(expected, str) and r.HASH.fullmatch(expected) is not None, "catalog_digest")
    actual = r.digest(path, 32 * 1024)
    r.require(actual["bytes"] > 0 and actual["sha256"] == expected, "catalog_source_binding")
    # Runtime validates semantic/native authority. This gate binds only the exact admitted bytes.
    marker = f"{expected} {actual['bytes']} {CATALOG}\n".encode("ascii")
    return {CATALOG: actual, CATALOG_HASH: blob(marker)}


def resources(archives: list[Path], expected: dict, *, prefix: str = "", forbid_catalog: bool = False) -> dict:
    wanted = {prefix + name: value for name, value in expected.items()}
    folds = {name.casefold(): name for name in wanted}
    forbidden = {CATALOG.casefold(), CATALOG_HASH.casefold()} if forbid_catalog else set()
    seen, archive_records = {}, []
    for path in archives:
        before = r.digest(path)
        r.jar_admission(path) if path.suffix == ".jar" else None
        with zipfile.ZipFile(path) as archive:
            names = set()
            for entry in archive.infolist():
                r.require(entry.filename not in names, "resource_archive_duplicate")
                names.add(entry.filename)
                folded = entry.filename.casefold()
                r.require(folded not in forbidden, "unexpected_catalog")
                if folded not in folds:
                    continue
                name = folds[folded]
                r.require(entry.filename == name and name not in seen and not entry.is_dir() and
                          not entry.flag_bits & 1 and entry.file_size == wanted[name]["bytes"], "resource_identity")
                sha, count = hashlib.sha256(), 0
                with archive.open(entry) as stream:
                    while chunk := stream.read(65536):
                        count += len(chunk)
                        r.require(count <= wanted[name]["bytes"], "resource_limit")
                        sha.update(chunk)
                actual = {"bytes": count, "sha256": sha.hexdigest()}
                r.require(actual == wanted[name], "resource_binding")
                seen[name] = actual
        r.require(r.digest(path) == before, "resource_archive_changed")
        archive_records.append({"name": path.name, **before})
    r.require(set(seen) == set(wanted), "resource_missing")
    return {"resources": seen, "archives": archive_records}


def runtime_inventory(inventory: dict) -> dict:
    runtime = {name: value for name, value in inventory.items() if name.startswith("runtime/")}
    required = {"runtime/bin/server/jvm.dll", "runtime/lib/modules",
                "runtime/legal/java.base/LICENSE", "runtime/legal/java.base/ADDITIONAL_LICENSE_INFO",
                "runtime/legal/java.base/ASSEMBLY_EXCEPTION"}
    r.require(required <= runtime.keys(), "runtime_legal_missing")
    legal = {name: value for name, value in runtime.items() if name.startswith("runtime/legal/")}
    r.require(all(value.get("bytes", 0) > 0 and r.HASH.fullmatch(value.get("sha256", "")) for value in legal.values()),
              "runtime_legal_binding")
    return {"files": runtime, "legal_files": legal,
            "project_exception": "unmodified jpackage Java runtime; preserve each upstream legal notice",
            "exception_inventory_kind": "actual delivered file hashes, not a license compatibility judgment"}


def notice_text(rows: list, runtime: dict) -> str:
    lines = ["Visual Workbench personal distribution - third-party notices", "",
             "Preserve the complete application image and its runtime/legal directory.",
             "The APK includes the listed notice resources under assets/; desktop JARs include them at resource root.", ""]
    for row in rows:
        lines.append(f"{row['resource']} | {row['license']} | SHA256 {row['sha256']} | {row['bytes']} bytes")
        lines.append(row["source_url"])
    lines += ["", "Bundled Java runtime exception inventory:", runtime["project_exception"]]
    lines += [f"{name} | SHA256 {value['sha256']} | {value['bytes']} bytes" for name, value in sorted(runtime["legal_files"].items())]
    lines += ["", "Full runtime file hashes are recorded in the adjacent package verification JSON.",
              "Package checks do not establish installed signature, startup, editor effects or physical acceptance.", ""]
    return "\n".join(lines)


def verify(root: Path, catalog_path: Path | None, catalog_sha: str | None) -> tuple[dict, str]:
    built = r.check(root)  # Existing exact six-native, nonce, source, runtime and MCP contract.
    expected, rows = notices(root)
    admitted = catalog(catalog_path, catalog_sha)
    image = root / r.DIST
    jars = sorted((image / "app").glob("*.jar"))
    desktop = resources(jars, expected | admitted, forbid_catalog=not admitted)
    apk = r.plain(root / "apps/android/build/outputs/apk/debug/android-debug.apk")
    module_path = root / "tools/ffi-test/check_apk.py"
    spec = importlib.util.spec_from_file_location("vw_personal_apk_check", module_path)
    r.require(spec is not None and spec.loader is not None, "apk_verifier")
    checker = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = checker
    spec.loader.exec_module(checker)
    try:
        native = checker.verify_apk(apk)
    except checker.Rejected as error:
        raise r.Rejected("apk_native_" + str(error)) from None
    android = resources([apk], expected, prefix="assets/")
    r.require(android["archives"][0]["sha256"] == native["apk_sha256"], "apk_changed")
    runtime = runtime_inventory(built["distribution"]["files"])
    # Do not combine an old build inventory with resources replaced during this check.
    r.require(r.distribution(root, built["nonce"]) == built["distribution"], "distribution_changed")
    report = {"schema": "M4-KP.personal-package.v1", "status": "passed-resource-verification",
              "desktop_build_nonce": built["nonce"], "desktop_build_receipt": r.digest(root / r.BUILT),
              "desktop": desktop, "android": android, "android_native": native,
              "catalog": {"present": bool(admitted), "resources": admitted, "action_authority_verified": False},
              "notices": rows, "notice_manifest": r.digest(root / "third_party/LICENSES"),
              "java_runtime": runtime, "source": {name: r.digest(root / name) for name in
                  ("tools/desktop-test/personal_package.py", "tools/desktop-test/reports.py", "tools/ffi-test/check_apk.py")},
              "android_source_build_verified": False,
              "application_id_verified": False, "signature_verified": False,
              "install_or_launch_verified": False, "editor_actions_verified": False, "physical_acceptance": False}
    return report, notice_text(rows, runtime)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--catalog", type=Path)
    parser.add_argument("--catalog-sha256")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--notice-index", type=Path, required=True)
    args = parser.parse_args()
    try:
        root = r.plain(args.root, True)
        local = r.plain(root / ".local", True)
        r.require(args.output.absolute() != args.notice_index.absolute(), "output_collision")
        r.require(all(path.absolute().parent == local for path in (args.output, args.notice_index)),
                  "outputs_require_local_receipt_directory")
        for path in (args.output, args.notice_index):
            r.relative_name(path.name)
            r.plain(path.parent, True)
            r.require(not path.exists(), "fresh_output_required")
        report, text = verify(root, args.catalog, args.catalog_sha256)
        # New output files only. Never modify package/runtime/source files.
        raw = text.encode("utf-8")
        r.require(len(raw) <= MAX_NOTICE_TOTAL, "notice_index_size")
        with args.notice_index.open("xb") as stream:
            stream.write(raw)
        report["notice_index"] = blob(raw)
        r.save(args.output, report)
        print("PERSONAL_PACKAGE_OK")
        return 0
    except (r.Rejected, OSError, ValueError, TypeError, KeyError, OverflowError, struct.error, zipfile.BadZipFile) as error:
        print("PERSONAL_PACKAGE_REFUSED:" + (str(error) if isinstance(error, r.Rejected) else "malformed_or_unavailable"))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
