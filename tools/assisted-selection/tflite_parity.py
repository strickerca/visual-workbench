#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Execute verified fixed graphs against private pinned-upstream golden tensors."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import resource

import verify_artifacts as verify


def sha256(path):
    state = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            state.update(block)
    return state.hexdigest()


def peak():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024


def compare(np, actual, expected, label, report):
    verify.require(actual.shape == expected.shape and np.isfinite(actual).all(), label + " shape/finite")
    difference = np.abs(actual.astype(np.float64) - expected.astype(np.float64))
    maximum = float(difference.max())
    report[label] = {"max_absolute_error": maximum, "reference_max_absolute": float(np.abs(expected).max()),
                     "shape": list(actual.shape)}
    verify.require(np.allclose(actual, expected, atol=1e-3, rtol=1e-3), label + " parity failure: max_abs=" + str(maximum))


def inventory(np, interpreter):
    rows = interpreter.get_tensor_details()
    bytes_sum = 0
    maximum = 0
    for row in rows:
        size = int(np.prod(row["shape"], dtype=np.int64)) * np.dtype(row["dtype"]).itemsize
        bytes_sum += size
        maximum = max(maximum, size)
    return {"tensor_count": len(rows), "sum_static_tensor_bytes": bytes_sum, "max_static_tensor_bytes": maximum}


def run(bundle, golden_path, output):
    import numpy as np
    from ai_edge_litert.interpreter import Interpreter, OpResolverType

    manifest_path = bundle / "model-manifest.json"
    manifest = verify.parse_canonical(verify.read_checked(bundle, manifest_path.name))
    manifest_hash = verify.verify_model(bundle, manifest)
    report = {"status": "host-cpu-parity-only", "host_runtime": "ai-edge-litert 2.2.0",
              "manifest_sha256": manifest_hash, "golden_sha256": sha256(golden_path),
              "graphs": {}, "cases": {}, "rss_peak_bytes": {}}
    with np.load(golden_path, allow_pickle=False) as golden:
        encoder = Interpreter(model_path=str(bundle / "encoder.tflite"), num_threads=2,
                              experimental_op_resolver_type=OpResolverType.BUILTIN_WITHOUT_DEFAULT_DELEGATES)
        encoder.allocate_tensors()
        report["graphs"]["encoder"] = inventory(np, encoder)
        report["rss_peak_bytes"]["encoder_allocated"] = peak()
        encoded = encoder.get_signature_runner("serving_default")(image=golden["image"])["image_embeddings"]
        compare(np, encoded, golden["image_embeddings"], "encoder", report["cases"])
        report["rss_peak_bytes"]["encoder_executed"] = peak()
        print("host encoder parity ok", flush=True)
        decoder = Interpreter(model_path=str(bundle / "prompt_decoder.tflite"), num_threads=2,
                              experimental_op_resolver_type=OpResolverType.BUILTIN_WITHOUT_DEFAULT_DELEGATES)
        decoder.allocate_tensors()
        report["graphs"]["prompt_decoder"] = inventory(np, decoder)
        report["rss_peak_bytes"]["both_allocated"] = peak()
        runner = decoder.get_signature_runner("serving_default")
        for name in ("no_prompts", "positive", "negative", "point_pair", "box", "mixed",
                     "all_padding_valid_zero", "all_padding_count_zero", "capacity_66"):
            value = runner(image_embeddings=golden["image_embeddings"],
                           point_coords=golden[name + ".point_coords"],
                           point_labels=golden[name + ".point_labels"],
                           point_valid=golden[name + ".point_valid"],
                           token_count=golden[name + ".token_count"])
            compare(np, value["mask_logits"], golden[name + ".mask_logits"], name + ".mask", report["cases"])
            compare(np, value["score"], golden[name + ".score"], name + ".score", report["cases"])
            print("host decoder parity " + name + " ok", flush=True)
        report["rss_peak_bytes"]["both_executed"] = peak()
    with output.open("x", encoding="ascii") as destination:
        json.dump(report, destination, sort_keys=True, separators=(",", ":"), allow_nan=False)
        destination.write("\n")
    print("TFLITE_PARITY_OK " + str(output), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--golden", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    run(arguments.bundle, arguments.golden, arguments.output)
