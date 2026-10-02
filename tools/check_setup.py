#!/usr/bin/env python3
"""Fail-closed, offline checks for starter policy files and retained requirement IDs.

Bundled binary entries in third_party/LICENSES are JSON objects containing path,
license, sha256, and source_url. No external binary exceptions are approved yet.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
ALLOWED = frozenset({
    "MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause",
    "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "Unicode-DFS-2016",
    "CC0-1.0", "MPL-2.0", "BSL-1.0", "OFL-1.1",
})
REQ_ID = re.compile(r"^[A-Z][A-Z0-9]*-\d{3}$")


def check_ids(root: pathlib.Path) -> list[str]:
    data = json.loads((root / "docs/REQUIREMENTS.json").read_text(encoding="utf-8-sig"))
    requirements = data["requirements"]
    ids = [row["id"] for row in requirements]
    issues = []
    if len(ids) != len(set(ids)):
        issues.append("REQUIREMENTS.json contains duplicate requirement IDs")
    if any(not REQ_ID.fullmatch(rid) for rid in ids):
        issues.append("REQUIREMENTS.json contains a malformed requirement ID")
    baseline = (root / "docs/v1_requirement_ids.txt").read_text(encoding="utf-8-sig").splitlines()
    baseline = [line.strip() for line in baseline if line.strip() and not line.lstrip().startswith("#")]
    if len(baseline) != 98 or len(set(baseline)) != 98:
        issues.append("v1_requirement_ids.txt must contain the 98 distinct original IDs")
    if any(not REQ_ID.fullmatch(rid) for rid in baseline):
        issues.append("v1_requirement_ids.txt contains a malformed requirement ID")
    missing = set(baseline) - set(ids)
    if missing:
        issues.append("Original requirement IDs missing: " + ", ".join(sorted(missing)))
    scenarios = data.get("scenarios", [])
    scenario_ids = [row["id"] for row in scenarios]
    if len(scenario_ids) != len(set(scenario_ids)):
        issues.append("REQUIREMENTS.json contains duplicate scenario IDs")
    for scenario in scenarios:
        unknown = set(scenario["requirements"]) - set(ids)
        if unknown:
            issues.append(f"{scenario['id']} references unknown requirement IDs: " + ", ".join(sorted(unknown)))
    return issues


def check_third_party(root: pathlib.Path) -> list[str]:
    third_party = root / "third_party"
    data = json.loads((third_party / "LICENSES").read_text(encoding="utf-8-sig"))
    if data.get("schema_version") != 1 or not isinstance(data.get("files"), list):
        return ["third_party/LICENSES must use schema_version 1 and a files list"]
    issues, listed = [], set()
    for entry in data["files"]:
        path = entry.get("path", "")
        relative = pathlib.PurePosixPath(path)
        if not path or relative.is_absolute() or ".." in relative.parts or "\\" in path or ":" in path:
            issues.append("third_party/LICENSES contains an unsafe relative path")
            continue
        if path in listed:
            issues.append(f"Duplicate third-party manifest path: {path}")
        listed.add(path)
        source = (third_party / path).resolve()
        if not source.is_relative_to(third_party.resolve()) or not source.is_file():
            issues.append(f"Bundled manifest file missing or outside third_party: {path}")
            continue
        if entry.get("license") not in ALLOWED:
            issues.append(f"Unapproved bundled binary license: {path}")
        if not isinstance(entry.get("source_url"), str) or not entry["source_url"].startswith("https://"):
            issues.append(f"Bundled manifest source_url missing: {path}")
        expected = entry.get("sha256", "")
        if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
            issues.append(f"Bundled manifest SHA-256 malformed: {path}")
        else:
            with source.open("rb") as contents:
                actual = hashlib.file_digest(contents, "sha256").hexdigest()
            if actual != expected:
                issues.append(f"Bundled manifest SHA-256 differs: {path}")
    present = {path.relative_to(third_party).as_posix() for path in third_party.rglob("*") if path.is_file() and path != third_party / "LICENSES"}
    for path in sorted(present - listed):
        issues.append(f"Unlisted bundled file: {path}")
    return issues


def check_workspace(root: pathlib.Path) -> list[str]:
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8-sig"))
    issues = []
    package_defaults = workspace.get("workspace", {}).get("package", {})
    expected_crates = {
        "core": {"vw-geom", "vw-model", "vw-ops", "vw-store", "vw-ink", "vw-raster", "vw-assets", "vw-pdf", "vw-svg", "vw-pkg", "vw-ai", "vw-proto", "vw-net", "vw-sim", "vw-ffi"},
        "host-win": {"vw-host-win", "vw-host-ffi"},
    }
    for component, names in expected_crates.items():
        for name in sorted(names):
            if not (root / component / "crates" / name / "Cargo.toml").is_file():
                issues.append(f"Required starter workspace crate is missing: {name}")
    for manifest in sorted((root / "core/crates").glob("*/Cargo.toml")) + sorted((root / "host-win/crates").glob("*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text(encoding="utf-8-sig"))["package"]
        private = package.get("publish")
        license_value = package.get("license")
        if isinstance(private, dict) and private.get("workspace"):
            private = package_defaults.get("publish")
        if isinstance(license_value, dict) and license_value.get("workspace"):
            license_value = package_defaults.get("license")
        if private is not False or license_value != "LicenseRef-VisualWorkbench-Proprietary":
            issues.append(f"Workspace crate must remain private and proprietary: {manifest.parent.name}")
    config = tomllib.loads((root / "deny.toml").read_text(encoding="utf-8-sig"))
    if set(config.get("licenses", {}).get("allow", [])) != ALLOWED:
        issues.append("deny.toml license allow-list differs from the approved starter list")
    if config.get("licenses", {}).get("private", {}).get("ignore") is not True:
        issues.append("deny.toml must exempt private workspace crates only")
    return issues


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("check", choices=["ids", "third-party", "workspace", "all"], default="all", nargs="?")
    parser.add_argument("--root", type=pathlib.Path, default=ROOT)
    args = parser.parse_args(argv)
    checks = {"ids": check_ids, "third-party": check_third_party, "workspace": check_workspace}
    selected = checks if args.check == "all" else {args.check: checks[args.check]}
    issues = []
    for name, check in selected.items():
        try:
            found = check(args.root.resolve())
            issues.extend(found)
            print(f"SETUP {name}: {'FAIL' if found else 'PASS'}")
        except (OSError, ValueError, KeyError, TypeError) as error:
            issues.append(f"{name}: required input invalid or unavailable ({type(error).__name__})")
            print(f"SETUP {name}: FAIL")
    for issue in issues:
        print(" - " + issue)
    return 1 if issues else 0


if __name__ == "__main__":
    sys.exit(main())
