#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Offline numerical parity of the fixed prompt wrapper and pinned MobileSAM."""
from __future__ import annotations

import argparse
import hashlib
import importlib
import json
import os
from pathlib import Path
import sys

import convert_mobile_sam as conversion
import verify_artifacts as verify


def maximum_error(actual, expected):
    delta = (actual - expected).abs()
    return float(delta.max().item())


def compare(torch, actual, expected, role, report, absolute=1e-4, relative=1e-4):
    verify.require(actual.shape == expected.shape and torch.isfinite(actual).all().item(), role + " shape/finite")
    error = maximum_error(actual, expected)
    report[role] = {"shape": list(actual.shape), "max_absolute_error": error,
                    "reference_max_absolute": float(expected.abs().max().item())}
    torch.testing.assert_close(actual, expected, atol=absolute, rtol=relative, msg=role)


def orientation(torch, value, code):
    if code == 1:
        return value
    if code == 2:
        return value.flip(-1)
    if code == 3:
        return value.flip((-2, -1))
    if code == 4:
        return value.flip(-2)
    if code == 5:
        return value.transpose(-2, -1)
    if code == 6:
        return torch.rot90(value, 3, (-2, -1))
    if code == 7:
        return value.transpose(-2, -1).flip((-2, -1))
    if code == 8:
        return torch.rot90(value, 1, (-2, -1))
    raise verify.VerificationError("unsupported orientation")


def prompt_case(torch, sam, wrapper, embedding, name, points, boxes, report,
                token_count=None, inactive=None, golden=None):
    device = embedding.device
    raw_points = None
    if points is not None:
        coords = torch.tensor([[row[:2] for row in points]], dtype=torch.float32, device=device)
        labels = torch.tensor([[row[2] for row in points]], dtype=torch.int32, device=device)
        raw_points = (coords, labels)
    raw_boxes = None if boxes is None else torch.tensor([boxes], dtype=torch.float32, device=device)
    sparse, dense = sam.prompt_encoder(raw_points, raw_boxes, None)
    expected_mask, expected_score = sam.mask_decoder(embedding, sam.prompt_encoder.get_dense_pe(),
        sparse, dense, multimask_output=False)

    slots = [] if points is None else list(points)
    if points is not None and boxes is None:
        slots.append((0.0, 0.0, -1))  # Upstream's required no-box sentinel.
    if boxes is not None:
        slots.extend(((boxes[0], boxes[1], 2), (boxes[2], boxes[3], 3)))
    verify.require(len(slots) <= 66, "prompt slot bound")
    coords = torch.zeros((1, 66, 2), dtype=torch.float32, device=device)
    labels = torch.full((1, 66), -1, dtype=torch.int32, device=device)
    valid = torch.zeros((1, 66), dtype=torch.int32, device=device)
    for index, (x, y, label) in enumerate(slots):
        coords[0, index] = torch.tensor((x, y), dtype=torch.float32, device=device)
        labels[0, index] = label
        valid[0, index] = 1
    # Poison ignored slots. They must not influence active output tokens.
    for index in range(len(slots), 66):
        coords[0, index] = torch.tensor((float(index * 7), float(index * 9)), device=device)
        labels[0, index] = index % 4
        valid[0, index] = 1 if inactive == "count" else 0
    count = len(slots) if token_count is None else token_count
    result = wrapper(embedding, coords, labels, valid, torch.tensor([count], dtype=torch.int32, device=device))
    compare(torch, result["mask_logits"], expected_mask, name + ".mask", report)
    compare(torch, result["score"], expected_score, name + ".score", report)
    if golden is not None:
        golden[name + ".point_coords"] = coords.cpu().numpy()
        golden[name + ".point_labels"] = labels.cpu().numpy()
        golden[name + ".point_valid"] = valid.cpu().numpy()
        golden[name + ".token_count"] = torch.tensor([count], dtype=torch.int32).numpy()
        golden[name + ".mask_logits"] = expected_mask.cpu().numpy()
        golden[name + ".score"] = expected_score.cpu().numpy()
    return result["mask_logits"]


def run(source_root, checkpoint, output, golden_path=None):
    conversion.revalidate_tool_snapshot(conversion.TOOLS_ROOT, conversion.STARTUP_TOOL_SNAPSHOT)
    catalog = conversion.conversion_catalog(conversion.STARTUP_TOOL_SNAPSHOT)
    sources = conversion.verify_source(source_root, catalog)
    conversion.verify_versions(catalog)
    pin = catalog["model"]["checkpoint"]
    report = {"status": "reference-parity-only", "model": "MobileSAM-vit_t",
              "source_commit": catalog["model"]["source_commit"], "checkpoint_sha256": pin["sha256"],
              "cases": {}}
    golden = {} if golden_path is not None else None
    with verify.checked_open(Path(checkpoint).absolute().parent, Path(checkpoint).name) as (stream, info):
        verify.require(info.st_size == pin["bytes"], "checkpoint byte pin")
        verify.require(hashlib.sha256(stream.read()).hexdigest() == pin["sha256"], "checkpoint hash pin")
        stream.seek(0)
        original = (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        def unchanged():
            current = os.fstat(stream.fileno())
            verify.require(original == (current.st_dev, current.st_ino, current.st_size,
                                        current.st_mtime_ns, current.st_ctime_ns), "checkpoint changed")
        finder = conversion.PinnedSource(sources, source_root)
        try:
            with conversion.offline_imports():
                torch = importlib.import_module("torch")
                conversion.verify_imported_cpu(torch, catalog)
                torch.set_num_threads(2)
                torch.manual_seed(0)
                sys.meta_path.insert(0, finder)
                builder = importlib.import_module("mobile_sam.build_sam")
                sam = builder.build_sam_vit_t(checkpoint=None)
                unchanged()
                state = torch.load(stream, map_location="cpu", weights_only=True, mmap=False)
                unchanged()
                verify.require(type(state) in (dict, __import__("collections").OrderedDict) and
                               all(type(key) is str and isinstance(value, torch.Tensor) for key, value in state.items()),
                               "checkpoint tensor state")
                sam.load_state_dict(state, strict=True)
                del state
                sam.train(True)
                sam.eval()
                encoder, decoder = conversion.build_wrappers(torch, sam)
                with torch.inference_mode():
                    original_rgb = torch.arange(320 * 480, dtype=torch.float32).reshape(1, 1, 320, 480)
                    original_rgb = torch.cat((original_rgb.remainder(251), (original_rgb / 7).remainder(251),
                                              (original_rgb / 23).remainder(251)), dim=1)
                    resized_height = int(320 * 1024 / 480 + 0.5)
                    resized = torch.nn.functional.interpolate(original_rgb, size=(resized_height, 1024),
                                                              mode="bilinear", align_corners=False)
                    image = sam.preprocess(resized)
                    upstream_embedding = sam.image_encoder(image)
                    embedding = encoder(image)["image_embeddings"]
                    compare(torch, embedding, upstream_embedding, "encoder", report["cases"], 0.0, 0.0)
                    if golden is not None:
                        golden["image"] = image.cpu().numpy()
                        golden["image_embeddings"] = upstream_embedding.cpu().numpy()
                    cases = (
                        ("no_prompts", None, None, 0, None),
                        ("positive", [(251.0, 311.0, 1)], None, None, None),
                        ("negative", [(251.0, 311.0, 0)], None, None, None),
                        ("point_pair", [(251.0, 311.0, 1), (619.0, 413.0, 0)], None, None, None),
                        ("box", None, (120.0, 160.0, 781.0, 730.0), None, None),
                        ("mixed", [(251.0, 311.0, 1), (619.0, 413.0, 0)], (120.0, 160.0, 781.0, 730.0), None, None),
                        ("all_padding_valid_zero", None, None, 66, None),
                        ("all_padding_count_zero", None, None, 0, "count"),
                        ("capacity_66", [(float(50+i*11), float(70+i*7), i%2) for i in range(64)],
                         (120.0, 160.0, 781.0, 730.0), 66, None),
                    )
                    exemplar = None
                    for name, points, boxes, count, inactive in cases:
                        print("reference case " + name, flush=True)
                        mask = prompt_case(torch, sam, decoder, embedding, name, points, boxes,
                                           report["cases"], count, inactive, golden)
                        if name == "mixed":
                            exemplar = mask
                    verify.require(exemplar is not None, "missing postprocess exemplar")
                    expected = sam.postprocess_masks(exemplar, (resized_height, 1024), (320, 480))
                    inverse = torch.nn.functional.interpolate(exemplar, size=(1024, 1024), mode="bilinear",
                                                              align_corners=False)[..., :resized_height, :1024]
                    inverse = torch.nn.functional.interpolate(inverse, size=(320, 480), mode="bilinear", align_corners=False)
                    compare(torch, inverse, expected, "inverse_resize", report["cases"], 0.0, 0.0)
                    for code in range(1, 9):
                        reverse = 8 if code == 6 else 6 if code == 8 else code
                        restored = orientation(torch, orientation(torch, inverse, code), reverse)
                        compare(torch, restored, inverse, "inverse_orientation_" + str(code), report["cases"], 0.0, 0.0)
                unchanged()
        finally:
            if finder in sys.meta_path:
                sys.meta_path.remove(finder)
            for name in list(sys.modules):
                if name == "mobile_sam" or name.startswith("mobile_sam."):
                    del sys.modules[name]
    output = Path(output)
    with output.open("x", encoding="utf-8") as destination:
        json.dump(report, destination, sort_keys=True, separators=(",", ":"), allow_nan=False)
        destination.write("\n")
    if golden_path is not None:
        import numpy as np
        with Path(golden_path).open("xb") as destination:
            np.savez_compressed(destination, **golden)
    print("REFERENCE_PARITY_OK " + str(output), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--golden", type=Path)
    arguments = parser.parse_args()
    run(arguments.source_root, arguments.checkpoint, arguments.output, arguments.golden)
