#!/usr/bin/env python3
"""Check the exact application graph census and its copied, hashed Maven POMs.

All artifacts must have an approved license. JNA is accepted only under its
documented Apache-2.0 option; strong copyleft and unknown artifacts fail.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sys
import xml.etree.ElementTree as ET

from check_setup import ALLOWED

ALIASES = {
    "The Apache Software License, Version 2.0": "Apache-2.0",
    "Apache License, Version 2.0": "Apache-2.0",
    "The Apache License, Version 2.0": "Apache-2.0",
    "Apache License Version 2.0": "Apache-2.0",
    "Apache License 2.0": "Apache-2.0",
    "Apache 2.0": "Apache-2.0",
    "Apache-2.0": "Apache-2.0",
    "MIT License": "MIT",
    "The MIT License": "MIT",
    "The MIT License (MIT)": "MIT",
    "BSD 2-Clause License": "BSD-2-Clause",
    "BSD 3-Clause License": "BSD-3-Clause",
    "Eclipse Public License 2.0": "EPL-2.0",
    "Mozilla Public License, Version 2.0": "MPL-2.0",
}
FORBIDDEN = re.compile(r"\b(?:A?GPL|LGPL|SSPL|BUSL|CC-BY-NC)(?:[-\s]|$)|General Public License|non[- ]commercial|unknown", re.I)
EXPECTED_CONFIGURATIONS = frozenset({
    ":android:debugRuntimeClasspath", ":android:releaseRuntimeClasspath",
    ":desktop:runtimeClasspath", ":shared:desktopRuntimeClasspath",
    ":pen-probe:debugRuntimeClasspath", ":pen-probe:releaseRuntimeClasspath",
    ":video-bench:debugRuntimeClasspath", ":video-bench:releaseRuntimeClasspath",
    ":image-bench:debugRuntimeClasspath", ":image-bench:releaseRuntimeClasspath",
    ":stroke-spike:debugRuntimeClasspath", ":stroke-spike:releaseRuntimeClasspath",
})


def read_pom_report(report: object, directory: pathlib.Path) -> dict:
    """Verify graph coverage and exact POM bytes before extracting license declarations.

    License names are explicit declarations, never inferred from module organization,
    URL, or a default. Inheritance uses only the exact retrieved/hash-bound parent
    chain, stopping at the nearest explicit declaration (maximum depth eight):
    https://maven.apache.org/pom.html#inheritance
    https://maven.apache.org/pom.html#licenses
    """
    if not isinstance(report, dict) or report.get("format") != "application-pom-census-v1":
        raise ValueError("Required application POM census format is missing")
    configurations = report.get("configurations")
    if not isinstance(configurations, dict) or set(configurations) != EXPECTED_CONFIGURATIONS:
        raise ValueError("Required Android, desktop, and shared runtime configuration coverage is incomplete")
    for name, coordinates in configurations.items():
        if (not isinstance(coordinates, list) or not coordinates
                or any(not isinstance(value, str) or value.count(":") != 2 for value in coordinates)
                or len(coordinates) != len(set(coordinates))):
            raise ValueError(f"Empty, duplicate, or invalid dependency census for {name}")
    expected = set().union(*(set(values) for values in configurations.values()))
    modules = report.get("dependencies")
    if not isinstance(modules, list) or not modules:
        raise ValueError("License census contains no dependency metadata")
    report_directory = directory.resolve(strict=True)

    def read_bound_pom(module: object) -> tuple[str, list[dict], str | None]:
        if not isinstance(module, dict):
            raise ValueError("Dependency metadata row must be an object")
        name, version = module.get("moduleName"), module.get("moduleVersion")
        if (not isinstance(name, str) or name.count(":") != 1 or not isinstance(version, str)
                or not version or ":" in version or any(not part for part in name.split(":"))):
            raise ValueError("Dependency metadata is missing exact module coordinates")
        key = f"{name}:{version}"
        expected_path = f"poms/{hashlib.sha256(key.encode('utf-8')).hexdigest()}.pom"
        if module.get("pomPath") != expected_path:
            raise ValueError(f"POM path is not bound to exact dependency coordinates: {key}")
        pom_file = (report_directory / expected_path).resolve(strict=True)
        if not pom_file.is_relative_to(report_directory) or not pom_file.is_file():
            raise ValueError(f"POM path escapes the report directory: {key}")
        data = pom_file.read_bytes()
        if module.get("pomSha256") != hashlib.sha256(data).hexdigest():
            raise ValueError(f"POM hash mismatch: {key}")
        if b"<!DOCTYPE" in data.upper() or b"<!ENTITY" in data.upper():
            raise ValueError(f"POM contains unsupported XML declarations: {key}")
        try:
            pom = ET.fromstring(data)
        except ET.ParseError as error:
            raise ValueError(f"POM XML is invalid: {key}") from error
        if pom.tag not in {"project", "{http://maven.apache.org/POM/4.0.0}project"}:
            raise ValueError(f"POM root is invalid: {key}")
        prefix = "{http://maven.apache.org/POM/4.0.0}" if pom.tag.startswith("{") else ""
        def value(path: str) -> str:
            element = pom.find("/".join(prefix + part for part in path.split("/")))
            return (element.text or "").strip() if element is not None else ""
        parent_rows = pom.findall(f"{prefix}parent")
        if len(parent_rows) > 1:
            raise ValueError(f"POM contains duplicate parent declarations: {key}")
        parent_key = None
        if parent_rows:
            parent_parts = []
            for tag in ("groupId", "artifactId", "version"):
                rows = parent_rows[0].findall(f"{prefix}{tag}")
                field = (rows[0].text or "").strip() if len(rows) == 1 else ""
                if not field or any(char.isspace() or char in ":${}" for char in field):
                    raise ValueError(f"POM parent coordinates are not literal and exact: {key}")
                parent_parts.append(field)
            parent_key = ":".join(parent_parts)
        group, artifact = name.split(":")
        if (value("modelVersion") != "4.0.0" or value("artifactId") != artifact
                or (value("groupId") or value("parent/groupId")) != group
                or (value("version") or value("parent/version")) != version):
            raise ValueError(f"POM coordinates do not match the resolved module: {key}")
        licenses = []
        if len(pom.findall(f"{prefix}licenses")) > 1:
            raise ValueError(f"POM contains duplicate license blocks: {key}")
        for row in pom.findall(f"{prefix}licenses/{prefix}license"):
            title = row.find(f"{prefix}name")
            url = row.find(f"{prefix}url")
            licenses.append({
                "moduleLicense": (title.text or "").strip() if title is not None else "",
                "moduleLicenseUrl": (url.text or "").strip() if url is not None else "",
            })
        return key, licenses, parent_key

    parent_rows = report.get("parentPoms", [])
    if not isinstance(parent_rows, list):
        raise ValueError("Parent POM metadata must be a list")
    parents = {}
    for row in parent_rows:
        key, licenses, parent = read_bound_pom(row)
        if key in parents:
            raise ValueError(f"Duplicate parent POM metadata: {key}")
        parents[key] = (licenses, parent)

    seen, referenced_parents = set(), set()
    annotated = []
    for module in modules:
        key, licenses, declared_parent = read_bound_pom(module)
        if key not in expected or key in seen:
            raise ValueError(f"Unexpected or duplicate dependency metadata: {key}")
        seen.add(key)
        source_configurations = sorted(config for config, keys in configurations.items() if key in keys)
        if module.get("configurations") != source_configurations:
            raise ValueError(f"Dependency configuration binding is incomplete: {key}")
        chain = module.get("parentPomChain", [])
        if (not isinstance(chain, list) or len(chain) > 8
                or any(not isinstance(value, str) or value.count(":") != 2 for value in chain)):
            raise ValueError(f"Parent POM chain is invalid or exceeds depth eight: {key}")
        chain_seen = {key}
        license_source = key
        for parent_key in chain:
            if parent_key in chain_seen:
                raise ValueError(f"Cycle in parent POM chain: {key}")
            chain_seen.add(parent_key)
            if licenses:
                raise ValueError(f"Parent POM chain continues beyond the nearest explicit license: {key}")
            if parent_key != declared_parent:
                raise ValueError(f"Parent POM chain does not match the explicit direct parent: {key}")
            if parent_key not in parents:
                raise ValueError(f"Parent POM metadata is missing: {parent_key}")
            referenced_parents.add(parent_key)
            licenses, declared_parent = parents[parent_key]
            license_source = parent_key
        annotated.append({**module, "moduleLicenses": licenses, "licenseSourcePom": license_source})
    if seen != expected:
        raise ValueError("Application graph and copied POM metadata census do not match")
    if referenced_parents != set(parents):
        raise ValueError("Parent POM census includes unused or missing chain metadata")
    return {"dependencies": annotated}


def find_modules(report: object) -> list[dict]:
    if not isinstance(report, dict):
        raise ValueError("Report root must be an object")
    found = []
    if "dependencies" in report:
        dependencies = report["dependencies"]
        if not isinstance(dependencies, list):
            raise ValueError("dependencies must be a list")
        found.extend(dependencies)
    for child in report.get("projects", []):
        found.extend(find_modules(child))
    if "dependencies" not in report and "projects" not in report:
        raise ValueError("Report contains no dependency or project list")
    return found


def check_report(report: object) -> tuple[int, list[str]]:
    modules = find_modules(report)
    issues = []
    if not modules:
        return 0, ["License report contains no dependencies; resolution or report generation is incomplete"]
    for module in modules:
        name = module.get("moduleName", "")
        version = module.get("moduleVersion", "")
        if not name or not version:
            issues.append("Dependency report contains a module without name or version")
            continue
        licenses = module.get("moduleLicenses", [])
        if not isinstance(licenses, list) or not licenses:
            issues.append(f"Missing dependency license: {name}:{version}")
            continue
        labels = [license_row.get("spdxLicense") or license_row.get("moduleLicense") or license_row.get("name") or license_row.get("title") or "" for license_row in licenses]
        canonical = [ALIASES.get(label.strip(), label.strip()) for label in labels if isinstance(label, str)]
        # JNA is dual-licensed: this project explicitly chooses Apache-2.0.
        if name in {"net.java.dev.jna:jna", "net.java.dev.jna:jna-platform"} and "Apache-2.0" in canonical:
            continue
        if len(canonical) != len(licenses) or any(not label or FORBIDDEN.search(label) for label in canonical):
            issues.append(f"Forbidden or unidentified dependency license: {name}:{version}")
        elif not all(label in ALLOWED for label in canonical):
            issues.append(f"Unapproved dependency license: {name}:{version} ({', '.join(canonical)})")
    return len(modules), issues


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=pathlib.Path)
    args = parser.parse_args(argv)
    try:
        report = json.loads(args.report.read_text(encoding="utf-8-sig"))
        count, issues = check_report(read_pom_report(report, args.report.parent))
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        print(f"GRADLE LICENSES: FAIL (required report invalid or unavailable: {type(error).__name__}: {error})")
        return 1
    print(f"GRADLE LICENSES: {'FAIL' if issues else 'PASS'} ({count} dependency entries)")
    for issue in issues:
        print(" - " + issue)
    return 1 if issues else 0


if __name__ == "__main__":
    sys.exit(main())
