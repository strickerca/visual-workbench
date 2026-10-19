#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bounded, offline census of the exact MobileSAM host conversion inputs."""
from __future__ import annotations

import argparse
from email.parser import Parser
import hashlib
import json
from pathlib import Path
import stat
import sys
import tarfile
from zipfile import BadZipFile, ZipFile


REPOSITORY = Path(__file__).resolve().parents[3]
LOCK = REPOSITORY / "tools/assisted-selection/host-wheel-lock.json"
SOURCES = Path(__file__).with_name("notice-sources.json")
CENSUS = Path(__file__).with_name("component-census.json")
PROPOSALS = Path(__file__).with_name("root-component-proposals.json")
NOTICE_PREFIX = "third_party/notices/assisted-selection/"
MODEL_SHA = "7ea1a2fe7c979305e65372d85cb3255e49434667cfaa1c636c8d2052518af9f4"
LOCK_SHA = "c0f62036a3dca78d52f3564399a7d657313143ce811a52aa47cd4b47633ffc17"
DELIVERY_SHA = "a97d339dd323f6045a6bf10c54c2ea9e687799023869cd655459c13866f1014f"
MAX_WHEEL = 256 * 1024 * 1024
MAX_ZIP_ENTRIES = 20_000
MAX_ZIP_EXPANDED = 2 * 1024 * 1024 * 1024
MAX_NOTICE = 4 * 1024 * 1024
MAX_MODEL = 128 * 1024 * 1024

SOURCE_TEXT = {
    "ai-edge-litert": ["upstream/LiteRT-v2.2.0-LICENSE"],
    "flatbuffers": ["upstream/flatbuffers-v25.12.19-LICENSE"],
    "litert-converter": [],  # No exact source-to-wheel build mapping was found.
    "litert-lm-builder": ["upstream/LiteRT-LM-v0.18.0-LICENSE"],
    "ordered-set": ["sdist/ordered-set-4.1.0/MIT-LICENSE"],
    "sentencepiece": [
        "sdist/sentencepiece-0.2.2/sentencepiece/LICENSE",
        "sdist/sentencepiece-0.2.2/sentencepiece/third_party/abseil-cpp/LICENSE",
        "sdist/sentencepiece-0.2.2/sentencepiece/third_party/darts_clone/LICENSE",
        "sdist/sentencepiece-0.2.2/sentencepiece/third_party/esaxx/LICENSE",
        "sdist/sentencepiece-0.2.2/sentencepiece/third_party/protobuf-lite/LICENSE",
    ],
    "tokenizers": ["sdist/tokenizers-0.23.3/tokenizers/LICENSE"],
}
SDISTS = {
    "ordered-set-4.1.0.tar.gz": "694a8e44c87657c59292ede72891eb91d34131f6531463aab3009191c77364a8",
    "sentencepiece-0.2.2.tar.gz": "3d2b5e824b5622038dc7b490897efe05ebbbb9e7350fc142f3ecc8789ef9bdf6",
    "tokenizers-0.23.3.tar.gz": "cded33237c77caeef62944d32aa9a7ef42bdce2b3497e18d137e072a8c4be438",
}


class AuditError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AuditError(message)


def digest(path: Path, maximum: int) -> tuple[int, str]:
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, "regular bounded file: " + str(path))
    state = hashlib.sha256()
    count = 0
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            count += len(chunk)
            require(count <= maximum, "file grew: " + str(path))
            state.update(chunk)
    require(count == info.st_size, "file changed: " + str(path))
    return count, state.hexdigest()


def stream_digest(source, maximum: int) -> tuple[int, str]:
    state = hashlib.sha256()
    count = 0
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        count += len(chunk)
        require(count <= maximum, "archive member limit")
        state.update(chunk)
    return count, state.hexdigest()


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                       allow_nan=False) + "\n").encode("ascii")


def read_json(path: Path, maximum: int = 1024 * 1024) -> dict:
    require(digest(path, maximum)[0] > 0, "empty JSON")
    raw = path.read_bytes()
    value = json.loads(raw)
    require(type(value) is dict, "JSON object")
    require(raw == canonical(value), "noncanonical JSON")
    return value


def archive_names(archive: ZipFile) -> dict:
    rows = archive.infolist()
    require(len(rows) <= MAX_ZIP_ENTRIES, "archive entry count")
    names = {}
    total = 0
    for row in rows:
        parts = row.filename.split("/")
        require(row.filename and not row.filename.startswith("/") and
                all(part not in ("", ".", "..") for part in parts[:-1]) and
                ".." not in parts, "unsafe archive path")
        require(row.filename not in names, "duplicate archive name")
        names[row.filename] = row
        total += row.file_size
        require(total <= MAX_ZIP_EXPANDED, "archive expansion limit")
    return names


def notice_name(name: str) -> bool:
    base = name.rsplit("/", 1)[-1].lower()
    return bool(base) and ("/licenses/" in name.lower() or base.startswith(
        ("license", "licence", "copying", "notice", "third_party_licenses")))


def license_text_candidate(record: dict) -> bool:
    # The inherited wheel inventory also counted Python modules below
    # directories named licenses. Those remain census entries, not notices.
    return record["bytes"] > 0 and not record["path"].lower().endswith(".py")


def native_name(name: str) -> bool:
    base = name.rsplit("/", 1)[-1]
    return ".so" in base or base.endswith((".dll", ".dylib", ".pyd"))


def verify_wheel(path: Path, row: dict) -> None:
    size, sha = digest(path, MAX_WHEEL)
    require((size, sha) == (row["bytes"], row["sha256"]), "wheel identity: " + row["distribution"])
    with ZipFile(path) as archive:
        names = archive_names(archive)
        native = {name for name in names if native_name(name)}
        notices = {name for name in names if notice_name(name)}
        require(native == {x["path"] for x in row["native_files"]}, "native census: " + row["distribution"])
        require(notices == {x["path"] for x in row["notices"]}, "notice census: " + row["distribution"])
        for record in row["native_files"] + row["notices"]:
            item = names[record["path"]]
            maximum = MAX_NOTICE if record in row["notices"] else MAX_ZIP_EXPANDED
            require(item.file_size == record["bytes"], "archive child bytes")
            with archive.open(item) as source:
                require(stream_digest(source, maximum) == (record["bytes"], record["sha256"]),
                        "archive child hash: " + record["path"])
        metadata = [item for name, item in names.items() if name.count("/") == 1 and
                    name.endswith(".dist-info/METADATA")]
        require(len(metadata) == 1 and metadata[0].file_size <= MAX_NOTICE, "wheel metadata")
        with archive.open(metadata[0]) as source:
            content = source.read(MAX_NOTICE + 1)
        require(hashlib.sha256(content).hexdigest() == row["metadata_sha256"], "metadata hash")
        fields = Parser().parsestr(content.decode("utf-8"))
        require(fields["Name"] == row["distribution"] and fields["Version"] == row["version"],
                "metadata distribution/version")


def verify_notice_sources(repo: Path, lock: dict, sources: dict) -> None:
    require(sources.get("schema") == "vw-assisted-notice-sources-v1", "notice source schema")
    records = sources.get("notices")
    require(type(records) is list and len(records) == 118, "notice source count")
    seen = set()
    by_path = {}
    for row in records:
        path = row["path"]
        require(path.startswith(NOTICE_PREFIX) and path not in seen and
                ".." not in Path(path).parts, "notice path")
        seen.add(path)
        by_path[path] = row
        actual = repo / path
        require(digest(actual, MAX_NOTICE) == (row["bytes"], row["sha256"]), "notice text identity: " + path)
        require(row["bytes"] > 0, "empty copied notice")
        actual.read_text(encoding="utf-8")
    expected = {NOTICE_PREFIX + "upstream/MobileSAM-f706ad9c-LICENSE"}
    for wheel in lock["wheels"]:
        for notice in wheel["notices"]:
            path = NOTICE_PREFIX + "host-wheel/" + notice["copy"]
            if not license_text_candidate(notice):
                require(path not in seen, "non-notice code/empty member copied as license text")
            else:
                expected.add(path)
                require(path in seen and by_path[path]["source_archive_sha256"] == wheel["sha256"] and
                        by_path[path]["sha256"] == notice["sha256"], "wheel notice source")
    for paths in SOURCE_TEXT.values():
        expected.update(NOTICE_PREFIX + path for path in paths)
    require(seen == expected, "missing/unexpected copied license or attribution text")
    actual_files = {str(path.relative_to(repo)) for path in (repo / NOTICE_PREFIX).rglob("*") if path.is_file()}
    require(actual_files == expected, "notice tree differs from inventory")
    require(sum(not license_text_candidate(row) for wheel in lock["wheels"]
                for row in wheel["notices"]) == 7, "non-license wheel member count")


def verify_sdist_sources(private: Path, repo: Path, sources: dict) -> None:
    grouped = {name: [] for name in SDISTS}
    for row in sources["notices"]:
        source = row["source"]
        if source.startswith("pypi-sdist:"):
            spec = source.removeprefix("pypi-sdist:")
            require(spec.count("!") == 1, "sdist source reference")
            name, member = spec.split("!", 1)
            require(name in grouped and row["source_archive_sha256"] == SDISTS[name], "sdist pin")
            grouped[name].append((member, row))
    require({name: len(rows) for name, rows in grouped.items()} ==
            {"ordered-set-4.1.0.tar.gz": 1, "sentencepiece-0.2.2.tar.gz": 5,
             "tokenizers-0.23.3.tar.gz": 1}, "sdist license inventory")
    for name, rows in grouped.items():
        path = private / "license-audit-inputs" / name
        require(digest(path, 10 * 1024 * 1024)[1] == SDISTS[name], "sdist bytes: " + name)
        with tarfile.open(path, "r:gz") as archive:
            members = archive.getmembers()
            require(len(members) <= MAX_ZIP_ENTRIES and
                    sum(item.size for item in members if item.isfile()) <= 128 * 1024 * 1024,
                    "sdist member bounds")
            names = [item.name for item in members]
            require(len(names) == len(set(names)) and
                    all(not name.startswith("/") and ".." not in Path(name).parts for name in names),
                    "unsafe/duplicate sdist member")
            for member, row in rows:
                item = archive.getmember(member)
                require(item.isfile() and item.size == row["bytes"], "sdist license bytes")
                source = archive.extractfile(item)
                require(source is not None and stream_digest(source, MAX_NOTICE) ==
                        (row["bytes"], row["sha256"]), "sdist license hash")
                require(digest(repo / row["path"], MAX_NOTICE) == (row["bytes"], row["sha256"]),
                        "sdist copied text")


def verify_proposals(sources: dict, proposals: dict) -> None:
    require(proposals.get("schema") == "vw-assisted-root-license-proposals-v1" and
            proposals.get("status") == "candidate-not-registered", "proposal status")
    rows = proposals.get("files")
    require(type(rows) is list and len(rows) == 11, "proposal count")
    source_rows = {row["path"].removeprefix("third_party/"): row for row in sources["notices"]
                   if not row["source"].startswith("wheel:")}
    require({row["path"] for row in rows} == set(source_rows), "proposed notice paths")
    for row in rows:
        require(set(row) == {"path", "license", "source_url", "sha256", "notice_scope"} and
                row["sha256"] == source_rows[row["path"]]["sha256"] and
                row["license"] in {"Apache-2.0", "MIT", "BSD-3-Clause"} and
                row["source_url"].startswith("https://") and
                ("unresolved" in row["notice_scope"] or
                 "unadmitted" in row["notice_scope"]), "proposal provenance")


def graph_metadata(path: Path, verifier) -> dict:
    data = path.read_bytes()
    fb = verifier.Flatbuffer(data)
    model = fb.target(0)
    names = []
    buffers = fb.tables(model, 4)
    for item in fb.tables(model, 6, cap=64):
        name = fb.string(item, 0)
        require(fb.scalar(item, 1) < len(buffers), "metadata buffer index")
        names.append(name)
    require(fb.string(model, 3) == "MLIR Converted." and
            sorted(names) == ["keep_stablehlo_constant", "min_runtime_version"],
            "unexpected model metadata")
    return {"description": fb.string(model, 3), "names": sorted(names),
            "embedded_license_or_notice": False}


def vendored_hint(name: str) -> bool:
    base = name.rsplit("/", 1)[-1]
    return ".libs/" in name or "/vendors/" in name or base.startswith((
        "libgomp", "libopenblas", "libquadmath", "libgfortran"))


def build_census(lock: dict, sources: dict, model: dict, metadata: dict, source_files: list) -> dict:
    source_by_path = {row["path"]: row for row in sources["notices"]}
    source_by_name = {row["path"]: row for row in source_files}
    tinyvit = source_by_name["mobile_sam/modeling/tiny_vit_sam.py"]
    sam_sources = [row for row in source_files if row["path"] not in
                   ("LICENSE", "mobile_sam/modeling/tiny_vit_sam.py")]
    wheels = []
    for row in lock["wheels"]:
        name = row["distribution"]
        recovered = [NOTICE_PREFIX + path for path in SOURCE_TEXT.get(name, [])]
        require(all(path in source_by_path for path in recovered), "source text missing: " + name)
        notices = [{"wheel_member": x["path"], "bytes": x["bytes"], "sha256": x["sha256"],
                    "license_or_attribution_text": license_text_candidate(x),
                    "repository_copy": (NOTICE_PREFIX + "host-wheel/" + x["copy"]
                                        if license_text_candidate(x) else None)}
                   for x in row["notices"]]
        native = [{"wheel_member": x["path"], "bytes": x["bytes"], "sha256": x["sha256"],
                   "possible_vendored_or_renamed": vendored_hint(x["path"]),
                   "build_and_component_license_review": "open"} for x in row["native_files"]]
        wheels.append({"distribution": name, "version": row["version"], "scope": "host-conversion-only",
                       "wheel": row["wheel"], "wheel_bytes": row["bytes"], "wheel_sha256": row["sha256"],
                       "metadata_sha256": row["metadata_sha256"],
                       "declared_license": row["license_metadata"],
                       "license_classifiers": [x for x in row["license_classifiers"] if x.startswith("License ::")],
                       "bundled_notices": notices, "recovered_source_text": recovered,
                       "native_files": native,
                       "review_state": ("source-text-only" if not notices and recovered else
                                        "missing-license-text" if not notices else
                                        "nonlicense-bundled-members" if not any(
                                            x["license_or_attribution_text"] for x in notices) else
                                        "bundled-text-unreviewed")})
    return {"schema": "vw-assisted-license-census-v1", "status": "incomplete-no-production-admission",
            "host_wheel_lock_sha256": LOCK_SHA, "delivery_manifest_sha256": DELIVERY_SHA,
            "model_manifest_sha256": MODEL_SHA,
            "totals": {"wheels": len(wheels), "native_files": sum(len(x["native_files"]) for x in wheels),
                       "wheel_notice_entries": sum(len(x["bundled_notices"]) for x in wheels),
                       "nonlicense_wheel_members": sum(not x["license_or_attribution_text"] for w in wheels
                                                       for x in w["bundled_notices"]),
                       "copied_nonempty_texts": len(sources["notices"])},
            "notices": sources["notices"], "host_wheels": wheels,
            "model": {"name": model["model"], "source_commit": model["source_commit"],
                      "source_files": source_files, "converter_files": model["converter"]["files"],
                      "third_party_components": [
                          {"name": "MobileSAM SAM-derived source", "files": sam_sources,
                           "license_evidence": NOTICE_PREFIX + "upstream/MobileSAM-f706ad9c-LICENSE",
                           "review_state": "pinned-root-text; attribution review open"},
                          {"name": "TinyViT architecture source", "files": [tinyvit],
                           "copyright_header": "Microsoft 2022; adapted from LeViT and Swin Transformer",
                           "review_state": "inherited-expression/notice mapping open"},
                          {"name": "LeViT architectural reference", "source_url":
                           "https://github.com/facebookresearch/levit",
                           "review_state": "referenced in TinyViT header; copied expression not established"},
                          {"name": "Swin Transformer architectural reference", "source_url":
                           "https://github.com/microsoft/swin-transformer",
                           "review_state": "referenced in TinyViT header; copied expression not established"},
                      ],
                      "checkpoint": {**model["checkpoint"], "license_status": "unresolved-separate-weights-terms"},
                      "converted_graphs": [{"role": a["role"], "bytes": a["bytes"], "sha256": a["sha256"],
                                            "metadata": metadata[a["role"]],
                                            "license_status": "derived-from-unresolved-checkpoint"}
                                           for a in model["artifacts"]],
                      "source_code_license": {"claim": "Apache-2.0 root text at pinned commit",
                                              "notice": NOTICE_PREFIX + "upstream/MobileSAM-f706ad9c-LICENSE",
                                              "tinyvit_attribution": "Microsoft 2022; LeViT/Swin adaptation; exact inherited obligations open"}},
            "android_runtime": {"version": "2.3.0", "scope": "separate-root-census",
                                "license_and_native_admission": "open"}}


def run(private: Path, write: bool) -> dict:
    lock_bytes, lock_sha = digest(LOCK, 1024 * 1024)
    require(lock_bytes > 0 and lock_sha == LOCK_SHA, "host lock changed")
    lock = read_json(LOCK)
    sources = read_json(SOURCES)
    verify_notice_sources(REPOSITORY, lock, sources)
    verify_sdist_sources(private, REPOSITORY, sources)
    verify_proposals(sources, read_json(PROPOSALS))
    require(len(lock["wheels"]) == 71 and len({x["distribution"] for x in lock["wheels"]}) == 71,
            "71 unique wheels")
    for row in lock["wheels"]:
        verify_wheel(private / "wheelhouse" / row["wheel"], row)
        for notice in row["notices"]:
            path = private / "artifacts/host-notices-v3" / notice["copy"]
            require(digest(path, MAX_NOTICE) == (notice["bytes"], notice["sha256"]),
                    "private wheel notice changed")
    require(sum(len(x["native_files"]) for x in lock["wheels"]) == 258 and
            sum(len(x["notices"]) for x in lock["wheels"]) == 114, "native/notice census")
    sys.path.insert(0, str(REPOSITORY / "tools/assisted-selection"))
    import verify_artifacts as verifier
    bundle = private / "artifacts/mobile-sam-fixed66"
    manifest_path = bundle / "model-manifest.json"
    require(digest(manifest_path, 1024 * 1024)[1] == MODEL_SHA, "model manifest changed")
    model = verifier.parse_canonical(verifier.read_checked(bundle, "model-manifest.json"))
    require(verifier.verify_model(bundle, model) == MODEL_SHA, "model artifact verification")
    metadata = {a["role"]: graph_metadata(bundle / a["path"], verifier) for a in model["artifacts"]}
    checkpoint = private / "inputs/mobile_sam.pt"
    require(digest(checkpoint, MAX_MODEL) == (model["checkpoint"]["bytes"],
                                             model["checkpoint"]["sha256"]), "checkpoint identity")
    with ZipFile(checkpoint) as archive:
        names = archive_names(archive)
        require(not any(notice_name(name) for name in names), "checkpoint embedded license inventory")
    source_catalog = verifier.load_catalog()["model"]
    require(source_catalog["source_commit"] == model["source_commit"], "source commit changed")
    for source in source_catalog["source_files"]:
        verifier.verify_child(private / "inputs/MobileSAM-pinned", source)
    require(digest(private / "artifacts/delivery-manifest-v3.json", 1024 * 1024)[1] == DELIVERY_SHA,
            "delivery manifest changed")
    census = build_census(lock, sources, model, metadata, source_catalog["source_files"])
    require(census["totals"] == {"wheels": 71, "native_files": 258, "wheel_notice_entries": 114,
                                 "nonlicense_wheel_members": 7, "copied_nonempty_texts": 118}, "census totals")
    if write:
        with CENSUS.open("xb") as output:
            output.write(canonical(census))
    else:
        require(digest(CENSUS, 2 * 1024 * 1024)[0] > 0 and CENSUS.read_bytes() == canonical(census),
                "committed census differs from exact bytes")
    return census


def require_closed(census: dict) -> None:
    require(census["status"] == "closed-production-admission", "license admission is open")
    require(all(wheel["review_state"] == "reviewed" for wheel in census["host_wheels"]),
            "host component review incomplete")
    require(census["model"]["checkpoint"]["license_status"] == "reviewed", "checkpoint terms unresolved")


def source_only() -> dict:
    """Check checked-in records when private model/wheel bytes are unavailable."""
    require(digest(LOCK, 1024 * 1024)[1] == LOCK_SHA, "host lock changed")
    lock = read_json(LOCK)
    sources = read_json(SOURCES)
    verify_notice_sources(REPOSITORY, lock, sources)
    verify_proposals(sources, read_json(PROPOSALS))
    census = read_json(CENSUS, 2 * 1024 * 1024)
    require(census.get("schema") == "vw-assisted-license-census-v1" and
            census.get("status") == "incomplete-no-production-admission" and
            census.get("host_wheel_lock_sha256") == LOCK_SHA and
            census.get("model_manifest_sha256") == MODEL_SHA and
            census.get("delivery_manifest_sha256") == DELIVERY_SHA, "source census header")
    require(census.get("totals") == {"wheels": 71, "native_files": 258,
                                      "wheel_notice_entries": 114, "nonlicense_wheel_members": 7,
                                      "copied_nonempty_texts": 118}, "source census totals")
    require(census.get("notices") == sources["notices"], "source census notice records")
    model = census.get("model", {})
    require(model.get("checkpoint") == {"bytes": 40728226,
            "sha256": "6dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f",
            "license_status": "unresolved-separate-weights-terms"}, "source checkpoint status")
    graphs = model.get("converted_graphs", [])
    require([(x["role"], x["bytes"], x["sha256"], x["license_status"]) for x in graphs] == [
        ("encoder", 28142464, "92f0ac2134bb88bb5e555cdb1dd7080bc4620fbe31790a799abcb4c26156f4d3",
         "derived-from-unresolved-checkpoint"),
        ("prompt_decoder", 18853624, "9c0f89743e7056c18f4ee771e299d7db848aa14b500c025b452851fb3e2dbb78",
         "derived-from-unresolved-checkpoint")], "source graph status")
    wheels = census.get("host_wheels")
    require(type(wheels) is list and len(wheels) == len(lock["wheels"]), "source census wheels")
    for wheel, original in zip(wheels, lock["wheels"]):
        require((wheel["distribution"], wheel["version"], wheel["wheel_sha256"],
                 wheel["wheel_bytes"], wheel["metadata_sha256"]) ==
                (original["distribution"], original["version"], original["sha256"],
                 original["bytes"], original["metadata_sha256"]), "source wheel identity")
        require([(x["wheel_member"], x["bytes"], x["sha256"]) for x in wheel["native_files"]] ==
                [(x["path"], x["bytes"], x["sha256"]) for x in original["native_files"]],
                "source native inventory")
        require([(x["wheel_member"], x["bytes"], x["sha256"]) for x in wheel["bundled_notices"]] ==
                [(x["path"], x["bytes"], x["sha256"]) for x in original["notices"]],
                "source wheel notice inventory")
        require(wheel["review_state"] != "reviewed", "unexpected license admission")
    return census


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--private-root", type=Path)
    parser.add_argument("--source-only", action="store_true", help="verify checked-in evidence without private bytes")
    parser.add_argument("--write-census", action="store_true", help="exclusive initial census creation")
    args = parser.parse_args()
    try:
        if args.source_only:
            require(not args.write_census, "source-only cannot write a census")
            result = source_only()
        else:
            require(args.private_root is not None, "--private-root required for full audit")
            result = run(args.private_root, args.write_census)
    except (AuditError, OSError, ValueError, KeyError, IndexError, TypeError,
            UnicodeError, BadZipFile, tarfile.TarError) as error:
        parser.exit(1, "LICENSE_AUDIT_REFUSED " + str(error) + "\n")
    print(("LICENSE_SOURCE_ONLY_OK " if args.source_only else "LICENSE_AUDIT_OK ") +
          json.dumps(result["totals"], sort_keys=True) +
          " production_admitted=false", flush=True)
