#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Close exact private host artifacts while leaving production admission closed."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import verify_artifacts as verify


def identity(path):
    state = hashlib.sha256()
    count = 0
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            state.update(block)
            count += len(block)
    return {"path": str(path), "bytes": count, "sha256": state.hexdigest()}


def main(private, tools, output):
    catalog = verify.load_catalog()
    bundle = private / "artifacts" / "mobile-sam-fixed66"
    model_manifest = verify.parse_canonical(verify.read_checked(bundle, "model-manifest.json"))
    model_hash = verify.verify_model(bundle, model_manifest, catalog)
    ref_path = private / "artifacts" / "reference-parity-golden.json"
    host_path = private / "artifacts" / "tflite-parity.json"
    golden_path = private / "artifacts" / "reference-golden.npz"
    reference = json.loads(ref_path.read_text())
    host = json.loads(host_path.read_text())
    verify.require(reference["source_commit"] == catalog["model"]["source_commit"] and
                   reference["checkpoint_sha256"] == catalog["model"]["checkpoint"]["sha256"],
                   "upstream reference identity")
    verify.require(host["manifest_sha256"] == model_hash and host["golden_sha256"] == identity(golden_path)["sha256"] and
                   host["host_runtime"] == "ai-edge-litert 2.2.0", "host parity identity")
    verify.require(len(host["cases"]) == 19 and
                   max(row["max_absolute_error"] for row in host["cases"].values()) <= 1e-3,
                   "host parity result")
    wheel_lock_path = tools / "host-wheel-lock.json"
    wheel_lock = json.loads(wheel_lock_path.read_text())
    verify.require(wheel_lock["python"] == "3.11.17" and len(wheel_lock["wheels"]) == 71,
                   "host wheel identity")
    wheelhouse = private / "wheelhouse"
    notice_root = private / "artifacts" / "host-notices-v3"
    for wheel in wheel_lock["wheels"]:
        found = identity(wheelhouse / wheel["wheel"])
        verify.require(found["bytes"] == wheel["bytes"] and found["sha256"] == wheel["sha256"],
                       "host wheel changed: " + wheel["distribution"])
        for notice in wheel["notices"]:
            found = identity(notice_root / notice["copy"])
            verify.require(found["bytes"] == notice["bytes"] and found["sha256"] == notice["sha256"],
                           "host notice changed: " + wheel["distribution"])
    input_root = private / "inputs"
    for source in catalog["model"]["source_files"]:
        verify.verify_child(input_root / "MobileSAM-pinned", source)
    checkpoint = identity(input_root / "mobile_sam.pt")
    pin = catalog["model"]["checkpoint"]
    verify.require(checkpoint["bytes"] == pin["bytes"] and checkpoint["sha256"] == pin["sha256"],
                   "checkpoint changed")
    files = [identity(bundle / name) for name in ("encoder.tflite", "prompt_decoder.tflite", "model-manifest.json")]
    files += [identity(path) for path in (ref_path, host_path, golden_path,
              wheel_lock_path, tools / "host-wheel-requirements.txt")]
    tool_files = [identity(tools / name) for name in ("convert_mobile_sam.py", "verify_artifacts.py",
                  "model-runtime-candidate.json", "reference_parity.py", "tflite_parity.py",
                  "lock_host_wheels.py", "make_delivery_manifest.py", "offline_exec.py",
                  "download_pinned_inputs.py")]
    execution_logs = [identity(private / "logs" / name) for name in (
        "input-download.log", "input-reverify.log", "wheel-download.log", "wheel-install.log",
        "seed-wheel-download.log", "reference-parity.log", "reference-parity-golden.log",
        "conversion.log", "conversion-2.log", "conversion-3.log", "tflite-parity.log",
        "offline-tools-tests.log", "host-lock-final.log")]
    graph_bytes = sum(row["bytes"] for row in model_manifest["artifacts"])
    tensors = sum(row["sum_static_tensor_bytes"] for row in host["graphs"].values())
    pool = 256 * 1024 * 1024
    image = 3 * 1024 * 1024 * 4
    embedding = 256 * 64 * 64 * 4
    known_envelope = tensors + graph_bytes + pool + image + embedding + 256 * 256 * 4
    missing_notices = [row["distribution"] for row in wheel_lock["wheels"] if not row["notices"]]
    payload = {
        "schema": "vw-assisted-host-delivery-v1",
        "status": "host-artifacts-closed-production-admission-open",
        "transfer_root": str(private),
        "model_manifest_sha256": model_hash,
        "source_commit": reference["source_commit"],
        "checkpoint": checkpoint,
        "artifacts_and_evidence": files,
        "tool_sources": tool_files,
        "execution_logs": execution_logs,
        "host_wheels": {"count": len(wheel_lock["wheels"]), "wheelhouse": str(wheelhouse),
                        "bundled_notice_count": sum(len(row["notices"]) for row in wheel_lock["wheels"]),
                        "native_file_count": sum(len(row["native_files"]) for row in wheel_lock["wheels"]),
                        "notice_root": str(notice_root), "missing_bundled_notices": missing_notices},
        "parity": {"upstream_case_count": len(reference["cases"]),
                   "host_case_count": len(host["cases"]),
                   "max_host_absolute_error": max(row["max_absolute_error"] for row in host["cases"].values()),
                   "host_runtime": host["host_runtime"]},
        "memory": {"host_peak_rss_bytes": host["rss_peak_bytes"]["both_executed"],
                   "static_tensor_sum_no_reuse_bytes": tensors,
                   "graph_file_bytes": graph_bytes,
                   "existing_original_publication_pool_bytes": pool,
                   "known_components_no_reuse_envelope_bytes": known_envelope,
                   "android_delegate_and_runtime_extra_bytes": "unmeasured; no complete Android bound"},
        "production_admissions": [],
        "remaining": ["seven host wheels lack bundled notice files; root must review source/native notices",
                      "Android LiteRT 2.3.0 CPU/GPU and packaged JNI/APK identities unrun",
                      "Android original/resized/snapshot/native/delegate working-set admission unrun",
                      "Android cancellation, correction/import/undo, segmentation quality and S23 latency unrun"],
    }
    with output.open("x", encoding="ascii") as destination:
        json.dump(payload, destination, sort_keys=True, separators=(",", ":"), allow_nan=False)
        destination.write("\n")
    print("DELIVERY_MANIFEST_OK " + str(output), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--private", type=Path, required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    main(args.private, args.tools, args.output)
