"""Independent, offline exterior verification. Requires the existing Pillow tool pin.

This implementation shares neither Rust dilation nor hashing code. It constructs
Euclidean disk dilation from integer row bitsets and checks decoded RGBA bytes.
It checks the original encoded-file hash; the normalized source PNG is the exact
document pixel baseline, avoiding cross-decoder JPEG rounding differences.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
from PIL import Image

MAX_PIXELS = 20_000_000
MAX_FILE = 100_000_000
Image.MAX_IMAGE_PIXELS = MAX_PIXELS


def bounded(path, maximum=MAX_FILE):
    if not path.is_file() or path.stat().st_size > maximum:
        raise ValueError("missing or oversized verification input")
    with path.open("rb") as stream:
        data = stream.read(maximum + 1)
    if len(data) > maximum:
        raise ValueError("verification input grew beyond limit")
    return data


def pixels(path):
    if path.stat().st_size > MAX_FILE:
        raise ValueError("encoded image exceeds verification limit")
    with Image.open(path) as image:
        w, h = image.size
        if image.format != "PNG" or image.mode != "RGBA" or not 0 < w <= 16384 or not 0 < h <= 16384 or w * h > MAX_PIXELS:
            raise ValueError("verification requires bounded RGBA PNG inputs")
        return w, h, image.tobytes()


def pixel_hash(w, h, rgba):
    return hashlib.sha256(b"vw-ai-spike/rgba8/v1\0" + struct.pack("<II", w, h) + rgba).hexdigest()


def expand_bits(bits, radius, width_mask):
    # OR all shifts [-radius,+radius], using doubling instead of pixel loops.
    run = bits
    distance = 1
    remaining = radius * 2
    while remaining:
        step = min(distance, remaining)
        run |= run << step
        remaining -= step
        distance *= 2
    return (run >> radius) & width_mask


def dilated_rows(mask, width, height, radius):
    rows = []
    for y in range(height):
        bits = 0
        for x, value in enumerate(mask[y * width:(y + 1) * width]):
            if value:
                bits |= 1 << x
        rows.append(bits)
    span = [(dy, math.isqrt(radius * radius - dy * dy)) for dy in range(-radius, radius + 1)]
    width_mask = (1 << width) - 1
    for y in range(height):
        result = 0
        for dy, half in span:
            if 0 <= y + dy < height:
                result |= expand_bits(rows[y + dy], half, width_mask)
        yield result


def verify(directory):
    proof = json.loads(bounded(directory / "proof.json", 65536))
    prepared = json.loads(bounded(directory / "prepared.json", 262144))
    description = prepared["description"]
    canonical = json.dumps(description, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
    if hashlib.sha256(canonical).hexdigest() != prepared["request_id"]:
        raise ValueError("request identity differs")
    w, h, source = pixels(directory / "source.png")
    rw, rh, result = pixels(directory / "composite.png")
    mw, mh, alpha_mask = pixels(directory / "mask.png")
    if (rw, rh) != (w, h) or (mw, mh) != (w, h) or (proof["width"], proof["height"]) != (w, h):
        raise ValueError("dimensions differ")
    radius = proof["dilation_px"]
    if type(radius) is not int or not 1 <= radius <= 65 or radius != description["feather_px"] + 1:
        raise ValueError("dilation radius differs")
    if proof["schema"] != 1 or proof["dilation_metric"] != "euclidean_disk_pixel_centers":
        raise ValueError("unknown proof definition")
    mask = bytes(255 - alpha_mask[index] for index in range(3, len(alpha_mask), 4))
    if not any(mask):
        raise ValueError("empty mask")
    mask_hash = hashlib.sha256(mask).hexdigest()
    if mask_hash != proof["mask_sha256"] or mask_hash != description["mask_sha256"]:
        raise ValueError("mask binding differs")
    source_hash = pixel_hash(w, h, source)
    if source_hash != proof["source_pixels_sha256"] or source_hash != description["source_pixels_sha256"]:
        raise ValueError("source pixel binding differs")
    with Image.open(directory / "source.png") as image:
        profile = image.info.get("icc_profile", b"")
    with Image.open(directory / "composite.png") as image:
        composite_profile = image.info.get("icc_profile", b"")
    if not profile or composite_profile != profile:
        raise ValueError("composite ICC binding differs")
    if hashlib.sha256(profile).hexdigest() != description["normalized_source_icc_sha256"]:
        raise ValueError("normalized source ICC binding differs")
    if description.get("source_icc_sha256") is not None:
        if hashlib.sha256(profile).hexdigest() != description["source_icc_sha256"]:
            raise ValueError("source ICC binding differs")
    if pixel_hash(w, h, result) != proof["result_pixels_sha256"]:
        raise ValueError("result pixel binding differs")
    original_hash = hashlib.sha256(bounded(directory / "original.input")).hexdigest()
    if original_hash != proof["source_file_sha256"] or original_hash != description["source_file_sha256"]:
        raise ValueError("immutable encoded-original binding differs")
    for filename, field in (("request-image.png", "request_image_sha256"), ("request-mask.png", "request_mask_sha256")):
        if hashlib.sha256(bounded(directory / filename)).hexdigest() != description[field]:
            raise ValueError("request payload binding differs")
    prefix = b"vw-ai-spike/outside/v1\0" + struct.pack("<III", w, h, radius) + mask_hash.encode("ascii")
    before = hashlib.sha256(prefix)
    after = hashlib.sha256(prefix)
    changed = outside = 0
    for y, excluded in enumerate(dilated_rows(mask, w, h, radius)):
        for x in range(w):
            if (excluded >> x) & 1:
                continue
            index = y * w + x
            a, b = source[index * 4:index * 4 + 4], result[index * 4:index * 4 + 4]
            outside += 1
            changed += a != b
            before.update(struct.pack("<Q", index)); before.update(a)
            after.update(struct.pack("<Q", index)); after.update(b)
    if (changed != proof["changed_outside"] or outside != proof["outside_pixels"]
            or before.hexdigest() != proof["outside_sha256_before"] or after.hexdigest() != proof["outside_sha256_after"]):
        raise ValueError("independently computed proof differs")
    if changed != 0 or before.digest() != after.digest():
        raise ValueError("outside the dilated mask pixels changed")
    return dict(independent_verification=True, changed_outside=changed, outside_pixels=outside,
                original_file_sha256=original_hash, outside_sha256=before.hexdigest(),
                request_id=prepared["request_id"], provider_semantics_verified=False)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(verify(args.directory), sort_keys=True))
    except (ValueError, KeyError, OSError, TypeError, Image.DecompressionBombError) as error:
        # Source paths, prompt text and private pixel content are never echoed.
        raise SystemExit("Independent exterior verification failed: " + type(error).__name__) from None
