#!/usr/bin/env python3
"""Generate THIRD-PARTY-NOTICES from the resolved, shipped dependency set (LIC-003).

Offline and read-only. Inputs are the resolved graphs the license gate already
admits; license texts are copied byte for byte from each component's own package
files (or from a hash-bound retained copy of those files). Nothing is downloaded,
built or installed, and no license text is written by this tool.

Shipped scope (everything else is excluded and listed in the receipt):

* Rust: `cargo metadata --locked --offline --all-features --filter-platform T`
  for each product target T. Walk only normal (non-dev, non-build) edges from
  the workspace crates that produce shipped binaries (RUST_ROOTS). Proc-macro
  crates run on the build host and are not traversed. First-party workspace
  crates are proprietary and excluded. Crate bytes come from the registry cache
  archive whose SHA-256 equals the Cargo.lock checksum.
* Gradle: the application census written by `generateApplicationLicenseReport`
  (the producer of `checkDependencyLicenses`), restricted to the product
  configurations in GRADLE_SHIPPING. Bench/probe/spike apps are excluded.
  Artifact bytes come from the read-only Gradle module cache and are checked
  against the module metadata SHA-256 (or the cache's SHA-1 directory name).
* npm: production (non-dev) entries of mcp/package-lock.json as admitted by
  tools/check_npm_licenses.py; texts are the reviewed, hash-bound notices.
* Bundled binaries: the Node runtime record and, when supplied, the jlink Java
  runtime legal directory of the built desktop image (documented exception).

Assembly (tools/release/assemble.py) consumes the text and receipt this tool
writes and re-verifies them with verify_bundle().
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tarfile
import zipfile

SCHEMA = "vw-generated-notices-v1"
TEXT_NAME = "THIRD-PARTY-NOTICES.txt"
RECEIPT_NAME = "third-party-notices.json"
MAX_TEXT = 4 * 1024 * 1024
MAX_TOTAL_TEXT = 48 * 1024 * 1024
MAX_ARCHIVE = 512 * 1024 * 1024
MAX_JSON = 32 * 1024 * 1024
HASH = re.compile(r"[0-9a-f]{64}\Z")
JAVA_RUNTIME = "java-runtime"
JAVA_EXCEPTION = "GPL-2.0 WITH Classpath-exception-2.0"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"

# Workspace crates whose binaries ship (build.ps1 native build lists and
# tools/desktop-test/reports.py NATIVES/MCP_REQUIRED; vw-bindgen is a build tool
# but shares vw-ffi, so its graph is already covered by the vw-ffi root).
RUST_ROOTS = {
    "x86_64-pc-windows-msvc": ("vw-ffi", "vw-host-ffi", "vw-capture", "vw-remote-host", "vw-mcp-native"),
    "aarch64-linux-android": ("vw-ffi",),
}
# Product application graphs; pen-probe/video-bench/image-bench/stroke-spike are
# measurement apps that no release artifact contains.
GRADLE_SHIPPING = (":android:debugRuntimeClasspath", ":android:releaseRuntimeClasspath",
                   ":desktop:runtimeClasspath", ":shared:desktopRuntimeClasspath")
NPM_SCOPE = "npm:mcp-production"
DESKTOP_COVERAGE = frozenset({"rust:x86_64-pc-windows-msvc", ":desktop:runtimeClasspath", NPM_SCOPE,
                              "bundled:node-runtime", "bundled:" + JAVA_RUNTIME})
ANDROID_COVERAGE = frozenset({"rust:aarch64-linux-android", ":android:debugRuntimeClasspath"})
FORBIDDEN = re.compile(r"\b(?:A?GPL|LGPL|SSPL|BUSL|CC-BY-NC)(?:[-\s]|$)|General Public License|non[- ]commercial", re.I)
NAME = re.compile(r"(?i)^(?:licen[cs]e|copying|copyright|notice|patents|unlicense|third[-_]?party[-_]?notices?)"
                  r"(?:[-_. ][a-z0-9._ -]*)?$|^[a-z0-9.]+[-_]licen[cs]e(?:\.[a-z]+)?$")
CODE = frozenset((".rs", ".java", ".kt", ".class", ".py", ".js", ".mjs", ".cjs", ".ts", ".c", ".h", ".cc",
                  ".cpp", ".go", ".json", ".toml", ".yml", ".yaml", ".html", ".xml", ".sh", ".ps1", ".bat",
                  ".in", ".cmake", ".sql", ".properties", ".kotlin_module", ".pom"))
ALWAYS = re.compile(r"(?i)^(?:notice|copyright|patents|third[-_]?party)")
SKIP_DIRS = frozenset(("test", "tests", "testdata", "examples", "example", "benches", "bench", "docs", "doc",
                       "fuzz", "ci", ".github", "target"))


class Refused(ValueError):
    """Static refusal code for assembly; never contains host paths."""


def require(ok: bool, code: str) -> None:
    if not ok:
        raise Refused(code)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def blob(data: bytes) -> dict:
    return {"bytes": len(data), "sha256": sha256(data)}


def ascii_text(value: str, limit: int = 4096) -> bool:
    return isinstance(value, str) and 0 < len(value) <= limit and all(32 <= ord(ch) < 127 for ch in value)


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    require(spec is not None and spec.loader is not None, "policy_unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


REPOSITORY = Path(__file__).absolute().parents[2]
# One allow list for every ecosystem: tools/check_setup.py (also enforces deny.toml).
SETUP = sys.modules.get("check_setup") or _load("check_setup", REPOSITORY / "tools/check_setup.py")
ALLOWED = frozenset(SETUP.ALLOWED)


# ---------------------------------------------------------------- SPDX policy
def spdx_dnf(expression: str) -> list[tuple[str, ...]]:
    """Parse an SPDX expression (plus the legacy crates.io "/" OR) into DNF."""
    require(ascii_text(expression, 512), "spdx_expression")
    tokens = re.findall(r"\(|\)|/|[A-Za-z0-9.+:-]+", expression)
    require("".join(tokens) == re.sub(r"\s+", "", expression), "spdx_expression")
    position = 0

    def peek():
        return tokens[position] if position < len(tokens) else None

    def take():
        nonlocal position
        require(position < len(tokens), "spdx_expression")
        position += 1
        return tokens[position - 1]

    def primary():
        token = peek()
        require(token is not None, "spdx_expression")
        if token == "(":
            take()
            value = disjunction()
            require(take() == ")", "spdx_expression")
            return value
        require(token not in (")", "/") and token.upper() not in ("AND", "OR", "WITH"), "spdx_expression")
        term = take()
        if peek() is not None and peek().upper() == "WITH":
            take()
            exception = take()
            require(exception not in ("(", ")", "/"), "spdx_expression")
            term = f"{term} WITH {exception}"
        return [(term,)]

    def conjunction():
        value = primary()
        while peek() is not None and peek().upper() == "AND":
            take()
            right = primary()
            value = [tuple(dict.fromkeys(a + b)) for a in value for b in right]
            require(len(value) <= 64, "spdx_expression")
        return value

    def disjunction():
        value = conjunction()
        while peek() is not None and (peek().upper() == "OR" or peek() == "/"):
            take()
            value = value + conjunction()
            require(len(value) <= 64, "spdx_expression")
        return value

    result = disjunction()
    require(position == len(tokens), "spdx_expression")
    return result


def family(term: str) -> str:
    base = term.split(" WITH ")[0].upper()
    for prefix, name in (("APACHE", "apache"), ("MIT", "mit"), ("BSD", "bsd"), ("0BSD", "isc"), ("ISC", "isc"),
                         ("ZLIB", "zlib"), ("UNICODE", "unicode"), ("MPL", "mpl"), ("BSL", "bsl"), ("CC0", "cc0"),
                         ("UNLICENSE", "unlicense"), ("OFL", "ofl"), ("AGPL", "gpl"), ("LGPL", "lgpl"), ("GPL", "gpl")):
        if base.startswith(prefix):
            return name
    return base.lower()


def detect(data: bytes) -> frozenset:
    """Coarse license-family detection, used only to pick among package files."""
    text = re.sub(r"\s+", " ", data[:65536].decode("utf-8", "replace")).lower()
    found = set()
    checks = (("apache", "apache license" in text and ("version 2.0" in text or "2.0" in text)),
              ("mit", "permission is hereby granted, free of charge" in text),
              ("bsd", "redistribution and use in source and binary forms" in text),
              ("isc", "permission to use, copy, modify, and/or distribute this software" in text or
               "permission to use, copy, modify, and distribute this software for any purpose" in text),
              ("zlib", "provided 'as-is'" in text or "provided \"as-is\"" in text or "provided as-is" in text),
              ("unicode", "unicode, inc" in text or "unicode license" in text),
              ("mpl", "mozilla public license" in text),
              ("bsl", "boost software license" in text),
              ("cc0", "cc0" in text or "creative commons legal code" in text),
              ("unlicense", "free and unencumbered software released into the public domain" in text),
              ("ofl", "sil open font license" in text),
              ("lgpl", "gnu lesser general public license" in text or "gnu library general public" in text),
              ("gpl", "gnu general public license" in text and "lesser" not in text[:400]))
    for name, hit in checks:
        if hit:
            found.add(name)
    return frozenset(found)


def term_allowed(term: str) -> bool:
    return term in ALLOWED


def selected_terms(selected: str) -> list[str]:
    terms = [part.strip() for part in selected.split(" AND ")]
    require(terms and all(ascii_text(term, 128) for term in terms), "selected_license")
    return terms


def leaf(origin: str) -> str:
    """File name of a text origin such as package:a.jar!META-INF/NOTICE."""
    path = origin.split(":", 1)[1] if ":" in origin.split("!", 1)[0] else origin
    return PurePosixPath(path.rsplit("!", 1)[-1]).name


def is_license_file(name: str) -> bool:
    base = PurePosixPath(name).name
    suffix = PurePosixPath(base).suffix.lower()
    return bool(NAME.match(base)) and suffix not in CODE


# ------------------------------------------------------------ resolution core
class Builder:
    def __init__(self, canonical: dict):
        self.components = {}
        self.texts = {}
        self.unresolved = []
        self.excluded = {}
        self.inputs = {}
        self.coverage = set()
        self.canonical = canonical  # spdx term -> (origin, bytes)

    def exclude(self, reason: str, item: str) -> None:
        self.excluded.setdefault(reason, set()).add(item)

    def text(self, data: bytes) -> str:
        require(0 < len(data) <= MAX_TEXT, "license_text_size")
        digest = sha256(data)
        self.texts[digest] = data
        require(sum(len(v) for v in self.texts.values()) <= MAX_TOTAL_TEXT, "license_text_total")
        return digest

    def add(self, ecosystem: str, name: str, version: str, declared: str, dnf, files: list, scope: str,
            sources: dict | None = None, extra: list | None = None, forced: str | None = None,
            notes: list | None = None, declaration_only: bool = False) -> None:
        """files: (origin, bytes) from the package; extra: (origin, bytes) always shipped."""
        key = (ecosystem, name, version)
        if key in self.components:
            self.components[key]["scope"].add(scope)
            return
        ident = f"{ecosystem}:{name}@{version}"
        for value in (name, version, declared):
            require(ascii_text(value, 512), "component_identity")
        extra = extra or []
        if forced is not None:
            dnf = [tuple(selected_terms(forced))]
        allowed = [branch for branch in dnf if all(term_allowed(t) for t in branch)]
        if not allowed:
            bad = sorted({t for branch in dnf for t in branch if not term_allowed(t)})
            kind = "forbidden_license" if any(FORBIDDEN.search(t) for t in bad) else "unknown_license"
            self.unresolved.append({"component": ident, "reason": kind, "detail": ", ".join(bad) or declared})
            return
        labelled = [(origin, data, detect(data)) for origin, data in files]
        retained_labels = [detect(data) for _, data in extra]
        present = set().union(*(labels for _, _, labels in labelled), *retained_labels)
        # Unrecognised LICENSE/COPYING files may hold the selected text; NOTICE,
        # COPYRIGHT and PATENTS files never stand in for a missing license text.
        unclassified = any(not labels and not ALWAYS.match(leaf(origin))
                           for origin, _, labels in labelled)
        choice, canonical, warnings = None, [], []
        for branch in allowed:
            if all(family(t) in present for t in branch):
                choice = branch
                break
        if choice is None:
            for branch in allowed:
                missing = [t for t in branch if family(t) not in present]
                if all(t in self.canonical for t in missing):
                    choice, canonical = branch, missing
                    warnings.append("package ships no license text for " + ", ".join(missing) +
                                    "; canonical text from a retained package file is used")
                    break
        if choice is None and declaration_only and labelled:
            # Owner-approved: upstream publishes no text for the declared license.
            # Record the declaration and ship every license file the package has.
            choice = allowed[0]
            warnings.append("declaration only: upstream publishes no " + " AND ".join(choice) +
                            " text; the package's declared license is recorded and its bundled license files ship")
        if choice is None and (unclassified or extra):
            choice = allowed[0]
            warnings.append("license text not matched to the selected license by content; all package license files are included")
        if choice is None:
            self.unresolved.append({"component": ident, "reason": "license_text_missing",
                                    "detail": "no package license file for " + " OR ".join(" AND ".join(b) for b in allowed)})
            return
        for note in notes or []:
            require(ascii_text(note, 1024), "supplement_note")
            warnings.append(note)
        families = {family(t) for t in choice}
        texts = []
        for origin, data, labels in labelled:
            if declaration_only or not labels or labels & families or ALWAYS.match(leaf(origin)):
                texts.append({"origin": origin, "sha256": self.text(data)})
        for origin, data in extra:
            texts.append({"origin": origin, "sha256": self.text(data)})
        for term in canonical:
            origin, data = self.canonical[term]
            texts.append({"origin": origin, "sha256": self.text(data)})
        require(texts, "license_text_missing")
        for row in texts:
            require(ascii_text(row["origin"], 1024), "text_origin")
        unique = {(row["origin"], row["sha256"]) for row in texts}
        self.components[key] = {
            "ecosystem": ecosystem, "name": name, "version": version, "declared": declared,
            "selected": " AND ".join(choice), "scope": {scope}, "sources": sources or {},
            "texts": [{"origin": o, "sha256": s} for o, s in sorted(unique)], "warnings": sorted(set(warnings)),
        }

    def finish(self) -> tuple[bytes, bytes, dict]:
        components = []
        for key in sorted(self.components):
            row = dict(self.components[key])
            row["scope"] = sorted(row["scope"])
            components.append(row)
        used = {t["sha256"] for row in components for t in row["texts"]}
        texts = {digest: self.texts[digest] for digest in sorted(used)}
        receipt = {
            "schema": SCHEMA, "generator": "tools/release/generate_notices.py",
            "status": "complete" if not self.unresolved else "incomplete",
            "inputs": dict(sorted(self.inputs.items())), "coverage": sorted(self.coverage),
            "components": components,
            "counts": {eco: sum(1 for c in components if c["ecosystem"] == eco)
                       for eco in sorted({c["ecosystem"] for c in components})},
            "excluded": {reason: sorted(items) for reason, items in sorted(self.excluded.items())},
            "unresolved": [dict(zip(("component", "reason", "detail"), row)) for row in
                           sorted({(r["component"], r["reason"], r["detail"]) for r in self.unresolved})],
            "texts": {digest: {"bytes": len(data)} for digest, data in texts.items()},
        }
        return finalize(receipt, texts)


def render(receipt: dict, texts: dict) -> bytes:
    """Deterministic text: a component index followed by each distinct text once."""
    out = [b"VISUAL WORKBENCH THIRD-PARTY NOTICES\n",
           ("Generated by " + receipt["generator"] + " (" + receipt["schema"] + ") from the resolved, "
            "shipped dependency set.\n").encode(),
           b"Each license text below is copied unchanged from the component's own package files;\n"
           b"an origin beginning with 'canonical:' marks a holder-independent text retained from another\n"
           b"package because the component ships none.\n\n",
           f"Components: {len(receipt['components'])}  Distinct license texts: {len(receipt['texts'])}\n".encode(),
           ("Coverage: " + ", ".join(receipt["coverage"]) + "\n\n").encode(), b"COMPONENT INDEX\n\n"]
    for row in receipt["components"]:
        lines = [f"[{row['ecosystem']}] {row['name']} {row['version']}",
                 f"    declared: {row['declared']}", f"    selected: {row['selected']}",
                 "    scope: " + ", ".join(row["scope"])]
        lines += [f"    text: {t['sha256']} {t['origin']}" for t in row["texts"]]
        lines += [f"    note: {w}" for w in row["warnings"]]
        out.append(("\n".join(lines) + "\n\n").encode())
    out.append(b"LICENSE TEXTS\n\n")
    for digest in sorted(receipt["texts"]):
        data = texts[digest]
        out.append(f"=== TEXT {digest} {len(data)} ===\n".encode())
        out.append(data)
        out.append(f"\n=== END TEXT {digest} ===\n\n".encode())
    return b"".join(out)


def finalize(receipt: dict, texts: dict) -> tuple[bytes, bytes, dict]:
    body = render(receipt, texts)
    receipt = dict(receipt)
    receipt["output"] = {"path": TEXT_NAME, **blob(body)}
    raw = (json.dumps(receipt, sort_keys=True, indent=2, ensure_ascii=True, allow_nan=False) + "\n").encode()
    return body, raw, receipt


# ------------------------------------------------------------- verification
def _strict_json(raw: bytes) -> dict:
    def unique(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value, "notices_json_duplicate")
            value[key] = item
        return value
    require(len(raw) <= MAX_JSON, "notices_receipt_size")
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique,
                           parse_constant=lambda _: (_ for _ in ()).throw(Refused("notices_json_constant")))
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        raise Refused("notices_receipt_structure") from error
    require(isinstance(value, dict), "notices_receipt_structure")
    return value


def verify_bundle(receipt_raw: bytes, text_raw: bytes, *, artifact_kinds=frozenset(),
                  declarations: dict | None = None) -> dict:
    """Re-check a generated bundle without regenerating it (used by assembly).

    The text must be exactly render(receipt, texts), every component must carry
    at least one text, every selected license must satisfy the allow list (or be
    the documented Java runtime exception), status must be complete, coverage
    must include every graph the delivered artifact kinds contain, and any input
    the release binding also declares must have identical bytes.
    """
    receipt = _strict_json(receipt_raw)
    keys = {"schema", "generator", "status", "inputs", "coverage", "components", "counts", "excluded",
            "unresolved", "texts", "output"}
    require(set(receipt) == keys and receipt["schema"] == SCHEMA, "notices_schema")
    require(receipt["status"] == "complete" and receipt["unresolved"] == [], "notices_incomplete")
    require(receipt["output"] == {"path": TEXT_NAME, **blob(text_raw)}, "notices_text_binding")
    require(isinstance(receipt["texts"], dict) and receipt["texts"], "notices_texts")
    texts, cursor = {}, 0
    marker = text_raw.find(b"LICENSE TEXTS\n\n")
    require(marker >= 0, "notices_text_structure")
    cursor = marker + len(b"LICENSE TEXTS\n\n")
    for digest in sorted(receipt["texts"]):
        require(HASH.fullmatch(digest) is not None, "notices_texts")
        size = receipt["texts"][digest].get("bytes") if isinstance(receipt["texts"][digest], dict) else None
        require(type(size) is int and 0 < size <= MAX_TEXT, "notices_texts")
        header = f"=== TEXT {digest} {size} ===\n".encode()
        require(text_raw[cursor:cursor + len(header)] == header, "notices_text_structure")
        cursor += len(header)
        data = text_raw[cursor:cursor + size]
        require(len(data) == size and sha256(data) == digest, "notices_text_hash")
        texts[digest] = data
        cursor += size
        trailer = f"\n=== END TEXT {digest} ===\n\n".encode()
        require(text_raw[cursor:cursor + len(trailer)] == trailer, "notices_text_structure")
        cursor += len(trailer)
    require(cursor == len(text_raw), "notices_text_structure")
    require(render(receipt, texts) == text_raw, "notices_text_render")
    require(isinstance(receipt["components"], list) and receipt["components"], "notices_components")
    used, seen = set(), set()
    for row in receipt["components"]:
        require(isinstance(row, dict) and {"ecosystem", "name", "version", "declared", "selected", "scope",
                                           "texts", "warnings", "sources"} == set(row), "notices_component")
        key = (row["ecosystem"], row["name"], row["version"])
        require(key not in seen and all(ascii_text(v, 512) for v in key), "notices_component")
        seen.add(key)
        require(row["texts"] and all(t["sha256"] in texts for t in row["texts"]), "notices_component_text")
        used.update(t["sha256"] for t in row["texts"])
        if row["ecosystem"] == "bundled" and row["name"] == JAVA_RUNTIME:
            require(row["selected"] == JAVA_EXCEPTION, "notices_forbidden_license")
        else:
            require(all(term_allowed(t) for t in selected_terms(row["selected"])), "notices_forbidden_license")
    require(used == set(texts), "notices_unused_text")
    coverage = set(receipt["coverage"])
    if "desktop-file" in artifact_kinds:
        require(DESKTOP_COVERAGE <= coverage, "notices_desktop_coverage")
    if "apk" in artifact_kinds:
        require(ANDROID_COVERAGE <= coverage, "notices_android_coverage")
    for path, expected in (declarations or {}).items():
        if path in receipt["inputs"]:
            require(receipt["inputs"][path] == expected, "notices_stale_input")
    return {"status": receipt["status"], "components": len(receipt["components"]),
            "counts": receipt["counts"], "coverage": sorted(coverage), "texts": len(texts),
            "output_sha256": receipt["output"]["sha256"]}


# ---------------------------------------------------------------- inputs
def read_file(path: Path, limit: int = MAX_ARCHIVE) -> bytes:
    path = Path(path)
    require(path.is_file() and not path.is_symlink(), "input_missing")
    data = path.read_bytes()
    require(len(data) <= limit, "input_size")
    return data


def retained(root: Path, relative: str, expected: str | None = None) -> bytes:
    """A curated notice must be registered in third_party/LICENSES with that hash."""
    require(relative.startswith("third_party/notices/") and ".." not in relative.split("/"), "retained_path")
    manifest = json.loads(read_file(root / "third_party/LICENSES", MAX_JSON))
    entry = next((row for row in manifest["files"] if "third_party/" + row["path"] == relative), None)
    require(entry is not None, "retained_unlisted")
    data = read_file(root / relative, MAX_TEXT)
    require(sha256(data) == entry["sha256"] and (expected is None or expected == entry["sha256"]), "retained_hash")
    return data


def load_supplements(root: Path, builder: Builder | None = None) -> dict:
    raw = read_file(root / "tools/licenses/notice-supplements.json", MAX_JSON)
    value = json.loads(raw)
    require(value.get("schema") == 1, "supplement_schema")
    if builder is not None:
        builder.inputs["tools/licenses/notice-supplements.json"] = blob(raw)
        builder.inputs["third_party/LICENSES"] = blob(read_file(root / "third_party/LICENSES", MAX_JSON))
    return value


def supplement_for(supplements: dict, ecosystem: str, name: str, version: str) -> dict | None:
    rows = [row for row in supplements.get("components", [])
            if (row["ecosystem"], row["name"], row["version"]) == (ecosystem, name, version)]
    require(len(rows) <= 1, "supplement_duplicate")
    return rows[0] if rows else None


def supplement_texts(root: Path, supplement: dict | None) -> list:
    if not supplement:
        return []
    return [("retained:" + n["path"], retained(root, n["path"], n["sha256"])) for n in supplement.get("notices", [])]


def canonical_texts(root: Path, supplements: dict) -> dict:
    found = {}
    for term, row in supplements.get("canonical_texts", {}).items():
        require(term in ALLOWED, "canonical_license")
        found[term] = ("canonical:" + term + ":" + row["path"], retained(root, row["path"], row["sha256"]))
    return found


# ------------------------------------------------------------------- Rust
def cargo_lock(root: Path) -> tuple[dict, bytes]:
    import tomllib
    raw = read_file(root / "Cargo.lock", MAX_JSON)
    lock = tomllib.loads(raw.decode("utf-8"))
    packages = {}
    for row in lock.get("package", []):
        packages[(row["name"], row["version"])] = row
    return packages, raw


def crate_files(cargo_home: Path, name: str, version: str, checksum: str | None, links: bool) -> tuple[list, dict]:
    prefix = f"{name}-{version}"
    archive = None
    for index in sorted((cargo_home / "registry/cache").glob("*")):
        candidate = index / (prefix + ".crate")
        if candidate.is_file():
            data = read_file(candidate)
            if checksum is not None and sha256(data) == checksum:
                archive = data
                break
    files = []
    depth = 3 if links else 1

    def wanted(parts: tuple) -> bool:
        return (len(parts) <= depth and is_license_file(parts[-1]) and
                not any(part.lower() in SKIP_DIRS for part in parts[:-1]))

    if archive is not None:
        with tarfile.open(fileobj=io.BytesIO(gzip.decompress(archive)), mode="r:") as tar:
            for member in tar.getmembers():
                parts = PurePosixPath(member.name).parts
                if len(parts) < 2 or parts[0] != prefix or not member.isfile() or not wanted(parts[1:]):
                    continue
                require(member.size <= MAX_TEXT, "license_text_size")
                stream = tar.extractfile(member)
                require(stream is not None, "crate_member")
                files.append(("package:" + "/".join(parts[1:]), stream.read()))
        return sorted(files), {"crate_sha256": checksum}
    # No archive whose SHA-256 equals the Cargo.lock checksum: never fall back to
    # unverified extracted sources; the caller records the crate as unresolved.
    return [], {}


def cargo_components(builder: Builder, root: Path, metadata: dict, cargo_home: Path,
                     supplements: dict | None = None) -> None:
    lock, raw = cargo_lock(root)
    builder.inputs["Cargo.lock"] = blob(raw)
    reached_any = set()
    for triple in sorted(metadata):
        require(triple in RUST_ROOTS, "rust_target")
        data = metadata[triple]
        packages = {row["id"]: row for row in data["packages"]}
        nodes = {row["id"]: row for row in data["resolve"]["nodes"]}
        members = set(data["workspace_members"])
        roots = [pid for pid in members if packages[pid]["name"] in RUST_ROOTS[triple]]
        require(len(roots) == len(RUST_ROOTS[triple]), "rust_roots")
        seen, stack = set(), list(roots)
        while stack:
            pid = stack.pop()
            if pid in seen:
                continue
            seen.add(pid)
            for dep in nodes[pid]["deps"]:
                if not any(kind.get("kind") is None for kind in dep["dep_kinds"]):
                    continue  # dev-dependencies and build-dependencies do not ship
                child = packages[dep["pkg"]]
                if any("proc-macro" in target["kind"] for target in child["targets"]):
                    builder.exclude("cargo:proc-macro-build-host-only", f"{child['name']} {child['version']}")
                    continue
                stack.append(dep["pkg"])
        scope = "rust:" + triple
        builder.coverage.add(scope)
        for pid in sorted(seen):
            package = packages[pid]
            name, version = package["name"], package["version"]
            reached_any.add((name, version))
            if pid in members:
                require(package.get("license") == "LicenseRef-VisualWorkbench-Proprietary", "first_party_license")
                builder.exclude("cargo:first-party-workspace", name)
                continue
            ident = f"cargo:{name}@{version}"
            if package.get("source") != CRATES_IO:
                builder.unresolved.append({"component": ident, "reason": "unsupported_source",
                                           "detail": str(package.get("source"))})
                continue
            entry = lock.get((name, version))
            if entry is None or not isinstance(entry.get("checksum"), str):
                builder.unresolved.append({"component": ident, "reason": "lock_mismatch", "detail": "not in Cargo.lock"})
                continue
            declared = package.get("license")
            if not declared:
                builder.unresolved.append({"component": ident, "reason": "unknown_license",
                                           "detail": "license-file only; needs review"})
                continue
            # Native-wrapping crates vendor C sources whose notices sit below the root.
            native = bool(package.get("links")) or name.endswith("-sys")
            files, sources = crate_files(cargo_home, name, version, entry["checksum"], native)
            if not sources:
                builder.unresolved.append({"component": ident, "reason": "crate_source_unavailable",
                                           "detail": "registry cache archive matching Cargo.lock checksum not found"})
                continue
            supplement = supplement_for(supplements or {}, "cargo", name, version)
            builder.add("cargo", name, version, declared, spdx_dnf(declared), files, scope, sources,
                        supplement_texts(root, supplement), supplement.get("selected") if supplement else None,
                        supplement.get("notes", []) if supplement else None,
                        bool(supplement and supplement.get("declaration_only") is True))
    for (name, version), row in lock.items():
        if (name, version) not in reached_any and row.get("source") == CRATES_IO:
            builder.exclude("cargo:not-in-shipped-graph(dev/build/other-target/tool)", f"{name} {version}")


def run_cargo_metadata(root: Path, toolchain: str) -> dict:
    found = {}
    for triple in sorted(RUST_ROOTS):
        # Metadata only: resolves the locked graph offline; compiles nothing.
        result = subprocess.run(["cargo", "+" + toolchain, "metadata", "--format-version", "1", "--locked",
                                 "--offline", "--all-features", "--filter-platform", triple],
                                cwd=root, capture_output=True, timeout=600, check=False)
        require(result.returncode == 0, "cargo_metadata_failed")
        found[triple] = json.loads(result.stdout)
    return found


# ------------------------------------------------------------------ Gradle
def _gradle_artifacts(cache: Path, group: str, module: str, version: str) -> tuple[list, str, list]:
    """Return ([(file name, bytes)], kind, redirect targets) for one coordinate.

    Only JVM/Android runtime library variants (org.gradle.usage java-runtime)
    reach an APK or desktop classpath; Kotlin metadata, other platforms and
    documentation variants are ignored. kind is "artifacts"; "redirect" when
    every runtime variant is published by another module (Gradle module
    metadata available-at); "platform" for POM-packaged BOM/platform modules;
    "no-artifact" when runtime variants only declare dependencies; or
    "absent" when the cache does not hold the bytes. The module metadata
    SHA-256, or the cache's SHA-1 directory name, binds every byte read.
    """
    base = cache / group / module / version
    if not base.is_dir():
        return [], "absent", []
    metadata = sorted(base.glob(f"*/{module}-{version}.module"))
    if metadata:
        value = json.loads(read_file(metadata[0], MAX_JSON))
        runtime = [variant for variant in value.get("variants", [])
                   if variant.get("attributes", {}).get("org.gradle.usage") == "java-runtime"
                   and variant.get("attributes", {}).get("org.gradle.category", "library") == "library"]
        if runtime and all("available-at" in variant for variant in runtime):
            targets = sorted({variant["available-at"]["group"] + ":" + variant["available-at"]["module"] + ":" +
                              variant["available-at"]["version"] for variant in runtime})
            return [], "redirect", targets
        wanted = {}
        for variant in runtime:
            for row in variant.get("files", []):
                wanted[row["name"]] = row.get("sha256")
        files = []
        for name, digest in sorted(wanted.items()):
            candidates = sorted(base.glob("*/" + name))
            if candidates:
                data = read_file(candidates[0])
                require(digest is None or sha256(data) == digest, "gradle_cache_hash")
                files.append((name, data))
        if files:
            return files, "artifacts", []
        if wanted:
            return [], "absent", []
        if runtime:
            # Runtime variants that only declare dependencies (e.g. JetBrains
            # aliases of AndroidX); the dependency itself is a census entry.
            return [], "no-artifact", []
    poms = sorted(base.glob(f"*/{module}-{version}.pom"))
    if poms and re.search(rb"<packaging>\s*pom\s*</packaging>", read_file(poms[0], MAX_JSON)):
        return [], "platform", []
    files = []
    for suffix in (".jar", ".aar"):
        for candidate in sorted(base.glob(f"*/{module}-{version}{suffix}")):
            data = read_file(candidate)
            require(hashlib.sha1(data).hexdigest() == candidate.parent.name, "gradle_cache_hash")
            files.append((candidate.name, data))
    return files, "artifacts" if files else "absent", []


NATIVE = (".so", ".dll", ".dylib", ".jnilib")
# Read-only byte probe of bundled native libraries for well-known embedded
# third-party code: (identifying byte strings, words a covering notice contains).
EMBEDDED = {
    "zlib": ((b"Jean-loup Gailly",), ("zlib", "jean-loup gailly")),
    "libpng": ((b"Glenn Randers-Pehrson", b"Cosmin Truta"), ("libpng",)),
    "libjpeg-turbo": ((b"libjpeg-turbo Project", b"Independent JPEG Group"), ("libjpeg",)),
    "FreeType": ((b"The FreeType Project", b"FT_Init_FreeType"), ("freetype",)),
    "HarfBuzz": ((b"HarfBuzz", b"harfbuzz"), ("harfbuzz",)),
    "ICU": ((b"icudt",), ("icu license", "unicode, inc", "international components for unicode")),
    "expat": ((b"XML_ParserCreate", b"expat_"), ("expat",)),
    "Wuffs": ((b"wuffs",), ("wuffs",)),
    "libwebp": ((b"WebPDecode", b"libwebp"), ("libwebp", "webm project")),
}


def native_blobs(data: bytes, depth: int = 0) -> list:
    found = []
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for entry in archive.infolist():
            if entry.filename.lower().endswith(NATIVE):
                require(entry.file_size <= MAX_ARCHIVE, "native_size")
                found.append(archive.read(entry))
            elif depth == 0 and entry.filename.endswith(".jar") and (entry.filename == "classes.jar" or
                                                                     entry.filename.startswith("libs/")):
                found += native_blobs(archive.read(entry), 1)
    return found


def embedded_components(files: list) -> list:
    detected = set()
    for _, data in files:
        for blob_bytes in native_blobs(data):
            for name, (signatures, _) in EMBEDDED.items():
                if any(signature in blob_bytes for signature in signatures):
                    detected.add(name)
    return sorted(detected)


def native_count(data: bytes, depth: int = 0) -> int:
    count = 0
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for entry in archive.infolist():
            if entry.filename.lower().endswith(NATIVE):
                count += 1
            elif depth == 0 and entry.filename.endswith(".jar") and (entry.filename == "classes.jar" or
                                                                     entry.filename.startswith("libs/")):
                count += native_count(archive.read(entry), 1)
    return count


def zip_license_files(name: str, data: bytes, depth: int = 0) -> list:
    found = []
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for entry in sorted(archive.infolist(), key=lambda e: e.filename):
            if entry.is_dir():
                continue
            if depth == 0 and (entry.filename == "classes.jar" or
                               (entry.filename.startswith("libs/") and entry.filename.endswith(".jar"))):
                require(entry.file_size <= MAX_ARCHIVE, "nested_archive_size")
                found += zip_license_files(name + "!" + entry.filename, archive.read(entry), 1)
                continue
            if is_license_file(entry.filename) and not entry.filename.startswith("res/"):
                require(entry.file_size <= MAX_TEXT, "license_text_size")
                found.append((f"package:{name}!{entry.filename}", archive.read(entry)))
    return found


def gradle_components(builder: Builder, root: Path, report_path: Path, cache: Path, supplements: dict) -> None:
    gradle = sys.modules.get("check_gradle_licenses") or _load("check_gradle_licenses",
                                                                REPOSITORY / "tools/check_gradle_licenses.py")
    raw = read_file(report_path, MAX_JSON)
    builder.inputs["apps/build/reports/dependency-license/dependencies.json"] = blob(raw)
    builder.inputs["tools/licenses/reviewed-maven.json"] = blob(read_file(root / "tools/licenses/reviewed-maven.json"))
    # The existing gate's reader binds every copied POM and its parent chain.
    annotated = gradle.read_pom_report(json.loads(raw), report_path.parent)["dependencies"]
    reviewed = {row["coordinate"]: row for row in json.loads(read_file(root / "tools/licenses/reviewed-maven.json"))["artifacts"]}
    for configuration in GRADLE_SHIPPING:
        builder.coverage.add(configuration)
    census = {m["moduleName"] + ":" + m["moduleVersion"] for m in annotated}
    for module in sorted(annotated, key=lambda m: (m["moduleName"], m["moduleVersion"])):
        name, version = module["moduleName"], module["moduleVersion"]
        key = f"{name}:{version}"
        scopes = sorted(set(module["configurations"]) & set(GRADLE_SHIPPING))
        if not scopes:
            builder.exclude("maven:non-product-configuration", key)
            continue
        group, artifact = name.split(":")
        files, kind, targets = _gradle_artifacts(cache, group, artifact, version)
        if kind == "redirect":
            if not any(target in census for target in targets):
                builder.unresolved.append({"component": f"maven:{key}", "reason": "redirect_target_missing",
                                           "detail": ", ".join(targets)})
                continue
            builder.exclude("maven:gradle-metadata-redirect(runtime bytes published by another listed module)", key)
            continue
        if kind == "platform":
            builder.exclude("maven:pom-packaged-platform-or-bom(no artifact)", key)
            continue
        if kind == "no-artifact":
            builder.exclude("maven:dependency-only-runtime-variant(no artifact)", key)
            continue
        labels = [row.get("spdxLicense") or row.get("moduleLicense") or "" for row in module["moduleLicenses"]]
        mapped = [gradle.ALIASES.get(label.strip(), label.strip()) for label in labels]
        declared = "; ".join(label.strip() for label in labels) or "none declared"
        supplement = supplement_for(supplements, "maven", name, version)
        forced = supplement.get("selected") if supplement else None
        if forced is not None:
            require(forced in mapped, "supplement_selection")
        if not mapped or any(not label for label in mapped):
            builder.unresolved.append({"component": f"maven:{key}", "reason": "unknown_license", "detail": declared})
            continue
        texts = []
        for file_name, data in files:
            texts += zip_license_files(file_name, data)
        retained_texts = supplement_texts(root, supplement)
        if module.get("licenseSourceReviewedArtifact") == key:
            retained_texts += [("retained:" + n["path"], retained(root, n["path"], n["sha256"]))
                               for n in reviewed[key]["notices"]]
        sources = {"artifacts": [{"name": n, "sha256": sha256(d)} for n, d in files]}
        if kind == "absent":
            sources["warning"] = "artifact bytes not present in the local Gradle cache; package files not inspected"
        natives = sum(native_count(data) for _, data in files)
        if natives:
            sources["native_libraries"] = natives
        reviewed_native = bool(supplement and ascii_text(supplement.get("native_review") or "", 1024))
        embedded = embedded_components(files) if natives else []
        if embedded:
            sources["embedded_native_components"] = embedded
            shipped = b"\n".join(data for _, data in texts + retained_texts).decode("utf-8", "replace").lower()
            # A generic text (e.g. plain Apache-2.0) names no project; a retained
            # notice registered under the library's name in third_party/LICENSES
            # covers it by that registered name.
            named = " ".join(leaf(origin) for origin, _ in retained_texts).lower()
            missing = [name for name in embedded
                       if not any(word in shipped for word in EMBEDDED[name][1]) and name.lower() not in named]
            if missing:
                builder.unresolved.append({"component": f"maven:{key}", "reason": "embedded_native_notice_missing",
                                           "detail": "bundled native code contains " + ", ".join(missing) +
                                                     " with no notice in the shipped texts"})
                continue
        if natives and not texts and not retained_texts and not reviewed_native:
            # A bundled native library may embed third-party code whose notice
            # the Maven license declaration does not cover; require a package
            # notice or a reviewed supplement instead of assuming none.
            builder.unresolved.append({"component": f"maven:{key}", "reason": "native_notice_unestablished",
                                       "detail": f"{natives} bundled native libraries and no package license file"})
            continue
        # Maven lists multiple <license> rows without operator semantics; the
        # existing gate requires each of them, so they are treated as AND.
        dnf = [tuple(dict.fromkeys(mapped))]
        for scope in scopes:
            builder.add("maven", name, version, declared if ascii_text(declared, 512) else "non-ascii declaration",
                        dnf, texts, scope, sources, retained_texts, forced,
                        (supplement.get("notes", []) + ([supplement["native_review"]] if reviewed_native else []))
                        if supplement else None)


# --------------------------------------------------------------------- npm
def npm_components(builder: Builder, root: Path, installed: Path | None) -> None:
    npm = sys.modules.get("check_npm_licenses") or _load("check_npm_licenses", REPOSITORY / "tools/check_npm_licenses.py")
    npm.verify(root)  # exact reviewed lock/policy/notice admission
    policy_raw = read_file(root / "tools/licenses/reviewed-npm.json", MAX_JSON)
    lock_raw = read_file(root / "mcp/package-lock.json", MAX_JSON)
    builder.inputs["tools/licenses/reviewed-npm.json"] = blob(policy_raw)
    builder.inputs["mcp/package-lock.json"] = blob(lock_raw)
    lock = json.loads(lock_raw)["packages"]
    builder.coverage.add(NPM_SCOPE)
    for row in sorted(json.loads(policy_raw)["packages"], key=lambda r: r["path"]):
        name = row["path"].removeprefix("node_modules/")
        if row["dev"] or lock[row["path"]].get("dev"):
            builder.exclude("npm:dev-only", f"{name} {row['version']}")
            continue
        files = []
        for notice in row["notices"]:
            data = retained(root, "third_party/" + notice["retained"], notice["sha256"])
            if installed is not None and (installed / row["path"] / notice["name"]).is_file():
                require(sha256(read_file(installed / row["path"] / notice["name"])) == notice["sha256"],
                        "npm_installed_notice")
            files.append(("package:" + name + "/" + notice["name"], data))
        builder.add("npm", name, row["version"], row["license"], spdx_dnf(row["license"]), files, NPM_SCOPE,
                    {"integrity": row["integrity"]})


# ---------------------------------------------------------------- bundled
def bundled_components(builder: Builder, root: Path, supplements: dict, desktop_image: Path | None) -> None:
    for row in supplements.get("bundled", []):
        record_raw = read_file(root / row["record"], MAX_JSON)
        record = json.loads(record_raw)
        builder.inputs[row["record"]] = blob(record_raw)
        require(record.get("signature_verified") is True, "bundled_record")
        data = retained(root, row["notice"]["path"], record["license_sha256"])
        scope = "bundled:" + row["id"]
        builder.coverage.add(scope)
        builder.add("bundled", row["id"], record["version"], row["license"], spdx_dnf(row["license"]),
                    [], scope, {"executable_sha256": record["node_sha256"]},
                    [("retained:" + row["notice"]["path"], data)])
    if desktop_image is None:
        return
    runtime = Path(desktop_image) / "runtime"
    release = read_file(runtime / "release", 64 * 1024).decode("utf-8", "replace")
    match = re.search(r'^JAVA_VERSION="([0-9A-Za-z.+_-]{1,64})"', release, re.M)
    require(match is not None, "java_runtime_release")
    legal = runtime / "legal"
    files = []
    for path in sorted(p for p in legal.rglob("*") if p.is_file()):
        relative = path.relative_to(Path(desktop_image)).as_posix()
        files.append(("package:" + relative, read_file(path, MAX_TEXT)))
    require(files, "java_runtime_legal_missing")
    scope = "bundled:" + JAVA_RUNTIME
    builder.coverage.add(scope)
    # Documented exception (BUILD_SPECIFICATION 2.5): unmodified jpackage runtime.
    key = ("bundled", JAVA_RUNTIME, match.group(1))
    texts = [{"origin": origin, "sha256": builder.text(data)} for origin, data in files]
    builder.components[key] = {"ecosystem": "bundled", "name": JAVA_RUNTIME, "version": match.group(1),
                               "declared": JAVA_EXCEPTION, "selected": JAVA_EXCEPTION, "scope": {scope},
                               "sources": {"release_sha256": sha256(release.encode())},
                               "texts": sorted(texts, key=lambda t: (t["origin"], t["sha256"])),
                               "warnings": ["documented exception: unmodified Java runtime bundled by jpackage"]}


# --------------------------------------------------------------------- CLI
def generate(root: Path, *, metadata: dict | None, cargo_home: Path, gradle_report: Path | None,
             gradle_cache: Path, npm_modules: Path | None, desktop_image: Path | None,
             skip: frozenset = frozenset()) -> tuple[bytes, bytes, dict]:
    builder = Builder({})
    supplements = load_supplements(root, builder)
    builder.canonical = canonical_texts(root, supplements)
    if "cargo" not in skip:
        cargo_components(builder, root, metadata or {}, cargo_home, supplements)
    if "gradle" not in skip:
        require(gradle_report is not None and Path(gradle_report).is_file(), "gradle_report_missing")
        gradle_components(builder, root, Path(gradle_report), gradle_cache, supplements)
    if "npm" not in skip:
        npm_components(builder, root, npm_modules)
    if "bundled" not in skip:
        bundled_components(builder, root, supplements, desktop_image)
    return builder.finish()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    home = Path(os.environ.get("USERPROFILE") or Path.home())
    parser.add_argument("--root", type=Path, default=REPOSITORY)
    parser.add_argument("--output", type=Path, required=True, help="fresh directory to create")
    parser.add_argument("--cargo-metadata", action="append", default=[], metavar="TRIPLE=FILE",
                        help="pre-captured `cargo metadata --locked --offline --all-features --filter-platform TRIPLE`")
    parser.add_argument("--cargo-toolchain", default="1.99.0")
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME") or home / ".cargo"))
    parser.add_argument("--gradle-report", type=Path, default=None,
                        help="dependencies.json written by generateApplicationLicenseReport "
                             "(default apps/build/reports/dependency-license/dependencies.json)")
    parser.add_argument("--gradle-cache", type=Path,
                        default=Path(os.environ.get("GRADLE_USER_HOME") or home / ".gradle") / "caches/modules-2/files-2.1")
    parser.add_argument("--npm-modules", type=Path, default=None, help="installed mcp/ directory (cross-check only)")
    parser.add_argument("--desktop-image", type=Path, default=None,
                        help="built jpackage app image (VisualWorkbenchDev) for the Java runtime legal files")
    parser.add_argument("--skip", action="append", default=[], choices=("cargo", "gradle", "npm", "bundled"))
    args = parser.parse_args(argv)
    root = args.root.absolute()
    skip = frozenset(args.skip)
    metadata = None
    if "cargo" not in skip:
        if args.cargo_metadata:
            metadata = {}
            for item in args.cargo_metadata:
                triple, _, path = item.partition("=")
                metadata[triple] = json.loads(read_file(Path(path), MAX_ARCHIVE))
        else:
            metadata = run_cargo_metadata(root, args.cargo_toolchain)
    report = args.gradle_report or root / "apps/build/reports/dependency-license/dependencies.json"
    try:
        text, receipt_raw, receipt = generate(root, metadata=metadata, cargo_home=args.cargo_home,
                                              gradle_report=report, gradle_cache=args.gradle_cache,
                                              npm_modules=args.npm_modules, desktop_image=args.desktop_image,
                                              skip=skip)
    except (Refused, OSError, ValueError, KeyError, TypeError, tarfile.TarError, zipfile.BadZipFile) as error:
        code = str(error) if isinstance(error, Refused) else type(error).__name__
        print("NOTICES_GENERATION_REFUSED:" + code)
        return 1
    args.output.mkdir()  # exclusive: never reuse an existing delivery directory
    (args.output / TEXT_NAME).write_bytes(text)
    (args.output / RECEIPT_NAME).write_bytes(receipt_raw)
    print(json.dumps({"status": receipt["status"], "counts": receipt["counts"],
                      "unresolved": len(receipt["unresolved"]), "coverage": receipt["coverage"],
                      "texts": len(receipt["texts"]), "output": receipt["output"],
                      "receipt": blob(receipt_raw)}, sort_keys=True))
    for row in receipt["unresolved"]:
        print(f"UNRESOLVED {row['component']} {row['reason']}: {row['detail']}")
    return 0 if receipt["status"] == "complete" else 2


if __name__ == "__main__":
    raise SystemExit(main())
