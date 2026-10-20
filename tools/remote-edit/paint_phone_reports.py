"""Bounded, offline evidence checks for the retained Paint phone-stroke parent.

This correlates source-owned receipts and exact canvas bytes. It grants no input,
process, image or file authority: the live parent must retain those native owners.
"""
import hashlib
import json
import math
import re
import struct
import sys
import zlib
from pathlib import Path

from controller_input_reports import validate_phone

MAX_INPUT = 1024 * 1024
MAX_PNG = 32 * 1024 * 1024
MAX_PIXELS = 4 * 1024 * 1024
U64 = (1 << 64) - 1
I64 = (1 << 63) - 1
SHA = re.compile(r"[a-f0-9]{64}\Z")
UUID = re.compile(r"[a-f0-9]{8}-[a-f0-9]{4}-7[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}\Z")
TOP_KEYS = set("schema target image paint_frame canvas_rect baseline after phone native_phone canvas_files stroke_ppm stroke_radius_pixels cursor_samples".split())
REASONS = set("arguments receipt_file receipt_bytes duplicate_json_key nonfinite_json json_shape envelope target image canvas witness witness_changed clock_order native_shape native_failure native_ownership native_identity native_scope native_metrics native_pixels phone_shape phone_failure phone_grant phone_retirement phone_binding phone_records png_file png_shape png_crc png_format png_compression png_hash decoded_hash metrics_mismatch baseline_unstable after_unstable baseline_not_white no_rgb_change stroke_shape outside_stroke_region cursor_shape cursor_changed cursor_inside_canvas unclassified".split())


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def uint(value, maximum=U64, minimum=0, reason="json_shape"):
    require(type(value) is int and minimum <= value <= maximum, reason)
    return value


def keys(value, required, optional=(), reason="json_shape"):
    require(isinstance(value, dict) and set(required) <= set(value) <= set(required) | set(optional), reason)


def digest(value, reason):
    require(isinstance(value, str) and SHA.fullmatch(value), reason)


def identifier(value, reason):
    require(isinstance(value, str) and UUID.fullmatch(value), reason)


def same(left, right):
    """JSON equality without Python's bool/int or int/float coercions."""
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(same(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(same(a, b) for a, b in zip(left, right))
    return left == right


def rect(value, reason="canvas"):
    keys(value, ("x", "y", "width", "height"), reason=reason)
    for axis in ("x", "y"):
        require(type(value[axis]) is int and -(1 << 31) <= value[axis] < (1 << 31), reason)
    for axis in ("width", "height"):
        uint(value[axis], 32768, 1, reason)
    require(value["x"] + value["width"] < (1 << 31) and value["y"] + value["height"] < (1 << 31), reason)
    return value


def inside(child, parent):
    return (child["x"] >= parent["x"] and child["y"] >= parent["y"]
            and child["x"] + child["width"] <= parent["x"] + parent["width"]
            and child["y"] + child["height"] <= parent["y"] + parent["height"])


def json_shape(value, depth=0):
    # Also protects callers of report(), which need not enter through json.loads.
    require(depth <= 32, "json_shape")
    if isinstance(value, dict):
        require(len(value) <= 256 and all(isinstance(k, str) and len(k) <= 128 for k in value), "json_shape")
        for item in value.values():
            json_shape(item, depth + 1)
    elif isinstance(value, list):
        require(len(value) <= 20000, "json_shape")
        for item in value:
            json_shape(item, depth + 1)
    elif isinstance(value, float):
        require(math.isfinite(value), "nonfinite_json")
    elif isinstance(value, str):
        require(len(value) <= 32768, "json_shape")
    else:
        require(value is None or type(value) in (bool, int), "json_shape")


def validate_target(target):
    keys(target, "token window process_id thread_id process_created window_rect frame_rect client_rect dpi integrity".split(), reason="target")
    identifier(target["token"], "target")
    for name in ("window", "process_created"):
        uint(target[name], U64, 1, "target")
    for name in ("process_id", "thread_id"):
        uint(target[name], (1 << 32) - 1, 1, "target")
    uint(target["dpi"], 768, 48, "target")
    uint(target["integrity"], (1 << 32) - 1, reason="target")
    for name in ("window_rect", "frame_rect", "client_rect"):
        rect(target[name], "target")
    require(inside(target["client_rect"], target["frame_rect"]) and inside(target["client_rect"], target["window_rect"]), "target")


def validate_image(image):
    keys(image, "executableName executableBlake3 executableBytes fileVersion packageFullName packageVersion".split(), reason="image")
    require(image["executableName"] == "mspaint.exe", "image")
    digest(image["executableBlake3"], "image")
    uint(image["executableBytes"], U64, 1, "image")
    require(image["fileVersion"] is None or isinstance(image["fileVersion"], str), "image")
    require(image["packageFullName"] == "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe"
            and image["packageVersion"] == "11.2605.81.0", "image")


WITNESS_KEYS = "settingsDigest canvasRuntimeIdHash canvasRectHost canvasToolName observedStartQpc100ns observedEndQpc100ns".split()


def witness(value, expected, canvas):
    keys(value, WITNESS_KEYS, reason="witness")
    for key in ("settingsDigest", "canvasRuntimeIdHash"):
        digest(value[key], "witness")
    require(value["canvasToolName"] == "Using Brush tool on Canvas", "witness")
    rect(value["canvasRectHost"])
    require(same(value["canvasRectHost"], canvas), "witness_changed")
    start = uint(value["observedStartQpc100ns"], I64 // 100, 1, "clock_order")
    end = uint(value["observedEndQpc100ns"], I64 // 100, 1, "clock_order")
    require(start < end, "clock_order")
    if expected is not None:
        require(all(value[k] == expected[k] for k in WITNESS_KEYS[:4]), "witness_changed")
    return start, end


METRIC_KEYS = "png_sha256 png_blake3 png_bytes width height canvas_pixels canvas_offset_x canvas_offset_y rgba_min rgba_max rgba_sum alpha_not_opaque_pixels decoded_rgba_sha256 explicit_srgb numeric_semantics_proved".split()
NATIVE_KEYS = "schema sampling_scope operation status helper_sha256 helper_pid helper_exit_code suspended_before_job_assignment exact_image_verified_before_resume process_and_job_retired io_worker_started io_join_completed error retirement_error io_error capture_refusal capture_color_depth target image source_proof capture_session_id sample_id before after frame canvas_frame canvas_metrics metrics diagnostic_sha256 input_sent profile_authority numeric_semantics_proved image_file_presence root_image_disposal_required".split()


def native_frame(receipt, target, image, expected, canvas, previous_end):
    keys(receipt, NATIVE_KEYS, ("installed_package_evidence", "capture_alpha_summary"), "native_shape")
    require(type(receipt["schema"]) is int and receipt["schema"] == 1
            and receipt["operation"] == "owned-blank-editor-wgc-metrics" and receipt["status"] == "Measured", "native_failure")
    require(receipt["sampling_scope"] == "verified_editor_canvas" and receipt["frame"] is None and receipt["metrics"] is None, "native_scope")
    for key in ("error", "retirement_error", "io_error", "capture_refusal", "capture_color_depth", "diagnostic_sha256", "capture_alpha_summary"):
        require(receipt.get(key) is None, "native_failure")
    for key in ("suspended_before_job_assignment", "exact_image_verified_before_resume", "process_and_job_retired", "io_worker_started", "io_join_completed", "image_file_presence", "root_image_disposal_required"):
        require(receipt[key] is True, "native_ownership")
    require(type(receipt["helper_exit_code"]) is int and receipt["helper_exit_code"] == 0, "native_failure")
    uint(receipt["helper_pid"], (1 << 32) - 1, 1, "native_ownership")
    digest(receipt["helper_sha256"], "native_identity")
    for key in ("input_sent", "profile_authority", "numeric_semantics_proved"):
        require(receipt[key] is False, "native_ownership")
    require(same(receipt["target"], target) and same(receipt["image"], image), "native_identity")
    proof = receipt["source_proof"]
    keys(proof, "policy held_process held_final_image held_ancestor_namespace installed_package_namespace namespace_lease_equivalence_claim input_authority profile_authority".split(), reason="native_identity")
    require(same(proof, {"policy": "installed_paint", "held_process": True, "held_final_image": True,
                      "held_ancestor_namespace": False, "installed_package_namespace": True,
                      "namespace_lease_equivalence_claim": False, "input_authority": False, "profile_authority": False}), "native_identity")
    package = receipt.get("installed_package_evidence")
    require(isinstance(package, dict) and package.get("full_name") == image["packageFullName"]
            and package.get("version") == image["packageVersion"] and package.get("status_ok") is True
            and type(package.get("status_flags")) is int and package["status_flags"] == 0
            and type(package.get("signature_kind")) is int and package["signature_kind"] == 3
            and package.get("development") is False and package.get("framework_or_resource_or_bundle_or_optional") is False, "native_identity")
    identifier(receipt["capture_session_id"], "native_identity")
    identifier(receipt["sample_id"], "native_identity")
    before_start, before_end = witness(receipt["before"], expected, canvas)
    after_start, after_end = witness(receipt["after"], expected, canvas)
    require(previous_end < before_start, "clock_order")
    frame = receipt["canvas_frame"]
    keys(frame, "source_identity target canvas_rect_host crop_in_frame source_asset_id png_bytes width height bit_depth border_visible lossless filename".split(), reason="native_shape")
    require(same(frame["canvas_rect_host"], canvas) and same(frame["width"], canvas["width"]) and same(frame["height"], canvas["height"]), "native_identity")
    require(frame["bit_depth"] == 8 and type(frame["bit_depth"]) is int and frame["border_visible"] is True
            and frame["lossless"] is True and frame["filename"] == "capture-canvas.png", "native_pixels")
    uint(frame["png_bytes"], MAX_PNG, 1, "native_pixels")
    digest(frame["source_asset_id"], "native_pixels")
    capture = frame["target"]
    keys(capture, "window process_id process_created client frame dpi observed_ns".split(), reason="native_shape")
    require(all(same(capture[k], target[k]) for k in ("window", "process_id", "process_created", "dpi"))
            and same(capture["client"], target["client_rect"]) and same(capture["frame"], target["frame_rect"]), "native_identity")
    observed = uint(capture["observed_ns"], I64, 1, "clock_order")
    crop = {"x": canvas["x"] - target["frame_rect"]["x"], "y": canvas["y"] - target["frame_rect"]["y"],
            "width": canvas["width"], "height": canvas["height"]}
    require(same(frame["crop_in_frame"], crop), "native_identity")
    identity = frame["source_identity"]
    keys(identity, "capture_session_id frame_id geometry_revision source_kind client_rect dpi_scale monotonic_timestamp_ns captured_at_ms platform window_handle monitor_id".split(), reason="native_shape")
    require(identity["capture_session_id"] == receipt["capture_session_id"] and type(identity["frame_id"]) is int and identity["frame_id"] == 1
            and type(identity["geometry_revision"]) is int and identity["geometry_revision"] == 1
            and identity["source_kind"] == "window" and identity["platform"] == "windows" and identity["monitor_id"] == ""
            and same(identity["window_handle"], target["window"]) and same(identity["client_rect"], target["client_rect"])
            and type(identity["dpi_scale"]) in (int, float) and identity["dpi_scale"] == target["dpi"] / 96, "native_identity")
    uint(identity["captured_at_ms"], I64, reason="clock_order")
    timestamp = uint(identity["monotonic_timestamp_ns"], I64, 1, "clock_order")
    require(before_end * 100 < timestamp < after_start * 100 and observed <= timestamp, "clock_order")
    metrics = receipt["canvas_metrics"]
    keys(metrics, "sampling_scope image source_client_rect canvas_rect_host crop_in_frame".split(), reason="native_metrics")
    require(metrics["sampling_scope"] == "verified_editor_canvas" and same(metrics["source_client_rect"], target["client_rect"])
            and same(metrics["canvas_rect_host"], canvas) and same(metrics["crop_in_frame"], crop), "native_scope")
    m = metrics["image"]
    keys(m, METRIC_KEYS, reason="native_metrics")
    for key in ("png_sha256", "png_blake3", "decoded_rgba_sha256"):
        digest(m[key], "native_metrics")
    require(m["png_blake3"] == frame["source_asset_id"] and same(m["png_bytes"], frame["png_bytes"]), "native_metrics")
    require(same(m["width"], canvas["width"]) and same(m["height"], canvas["height"]) and same(m["canvas_pixels"], canvas["width"] * canvas["height"])
            and type(m["canvas_offset_x"]) is int and m["canvas_offset_x"] == 0
            and type(m["canvas_offset_y"]) is int and m["canvas_offset_y"] == 0
            and m["explicit_srgb"] is True and m["numeric_semantics_proved"] is False, "native_metrics")
    for key, maximum in (("rgba_min", 255), ("rgba_max", 255), ("rgba_sum", MAX_PIXELS * 255)):
        require(isinstance(m[key], list) and len(m[key]) == 4, "native_metrics")
        for value in m[key]:
            uint(value, maximum, reason="native_metrics")
    require(type(m["alpha_not_opaque_pixels"]) is int and m["alpha_not_opaque_pixels"] == 0
            and m["rgba_min"][3] == m["rgba_max"][3] == 255 and m["rgba_sum"][3] == m["canvas_pixels"] * 255, "native_pixels")
    return after_end, m


def phone_receipts(phone, native, target):
    keys(phone, "schema status native_receipt_path native_receipt_sha256 contract_sha256 actual_remote_retirement retained_parent_returned receiver_observed editor_effect local_history_may_begin physical_pen_fidelity latency_acceptance started_qpc_100ns completed_qpc_100ns".split(), ("stroke_ppm",), reason="phone_shape")
    require(type(phone["schema"]) is int and phone["schema"] == 1 and phone["status"] == "phone-stroke-correlated", "phone_failure")
    for key in ("native_receipt_sha256", "contract_sha256"):
        digest(phone[key], "phone_shape")
    require(isinstance(phone["native_receipt_path"], str) and 0 < len(phone["native_receipt_path"]) <= 32768, "phone_shape")
    for key in ("actual_remote_retirement", "retained_parent_returned", "local_history_may_begin"):
        require(phone[key] is True, "phone_retirement")
    for key in ("receiver_observed", "editor_effect", "physical_pen_fidelity", "latency_acceptance"):
        require(phone[key] is False, "phone_failure")
    require(isinstance(native, dict) and type(native.get("schema")) is int and native["schema"] == 1
            and native.get("kind") == "normal-native-remote-usb-integration" and native.get("status") == "passed"
            and native.get("model") == "SM-S918U" and type(native.get("owner_user")) is int and native["owner_user"] == 0
            and native.get("controller_input_scenario") == "balanced", "phone_failure")
    require(isinstance(native.get("run_id"), str) and re.fullmatch(r"[a-f0-9]{32}", native["run_id"])
            and native.get("retained_input_nonce") == native["run_id"]
            and native.get("owned_editor_contract_sha256") == phone["contract_sha256"], "phone_binding")
    for key in ("controller_input_requested", "input_grant_attempted", "input_granted", "input_attempted", "input_dispatch_completed", "software_generated_input"):
        require(native.get(key) is True, "phone_grant")
    for key in ("input_retirement_confirmed", "host_actual_retirement", "phone_actual_retirement", "process_cleanup", "owned_packages_cleanup", "private_work_disposed", "external_editor_destination_retained", "profiles_preserved"):
        require(native.get(key) is True, "phone_retirement")
    for key in ("editor_effect", "physical_pen_fidelity", "latency_acceptance", "controller_input_records_validated", "held_lifecycle_records_validated"):
        require(native.get(key) is False, "phone_failure")
    for key in ("editor_phone_records_validated", "phone_render_records_validated"):
        require(native.get(key) is True, "phone_records")
    for key in ("failure_phase", "failure_error_type", "failure_error_code", "host_grant_failure", "host_first_run_failure", "host_first_carrier_failure"):
        require(native.get(key) is None, "phone_failure")
    require(native.get("held_lifecycle_records") == [], "phone_records")
    try:
        validate_phone(native["controller_input_records"], native["phone_render_records"])
    except (ValueError, KeyError, TypeError, OverflowError):
        raise ValueError("phone_records") from None
    # The remote selector mints its own target token. Its contract hash binds
    # the retained parent/window; that token is not the probe's snapshot token.
    start = uint(phone["started_qpc_100ns"], I64 // 100, 1, "clock_order")
    end = uint(phone["completed_qpc_100ns"], I64 // 100, 1, "clock_order")
    require(start < end, "clock_order")
    return start, end


def read_bytes(path, limit, reason):
    require(isinstance(path, str) and 0 < len(path) <= 32768, reason)
    p = Path(path)
    require(p.is_file() and not p.is_symlink() and 0 < p.stat().st_size <= limit, reason)
    with p.open("rb") as stream:
        raw = stream.read(limit + 1)
    require(0 < len(raw) <= limit, reason)
    return raw


def paeth(a, b, c):
    p = a + b - c
    distances = (abs(p - a), abs(p - b), abs(p - c))
    return (a, b, c)[distances.index(min(distances))]


def decode_png(raw, width, height):
    """Decode one bounded, nonanimated RGBA8 image with an explicit sRGB chunk."""
    require(0 < width <= 4096 and 0 < height <= 4096 and width * height <= MAX_PIXELS, "png_format")
    require(len(raw) <= MAX_PNG and raw[:8] == b"\x89PNG\r\n\x1a\n", "png_shape")
    at, chunks, compressed, srgb, ended = 8, 0, bytearray(), False, False
    idat_seen, idat_ended = False, False
    while at < len(raw):
        require(not ended and len(raw) - at >= 12, "png_shape")
        length = struct.unpack_from(">I", raw, at)[0]
        kind = raw[at + 4:at + 8]
        require(length <= MAX_PNG and at + 12 + length <= len(raw) and all(65 <= c <= 90 or 97 <= c <= 122 for c in kind), "png_shape")
        data = raw[at + 8:at + 8 + length]
        crc = struct.unpack_from(">I", raw, at + 8 + length)[0]
        require(zlib.crc32(kind + data) & 0xffffffff == crc, "png_crc")
        at += 12 + length
        chunks += 1
        require(chunks <= 4096 and (chunks != 1 or kind == b"IHDR"), "png_shape")
        if kind == b"IHDR":
            require(chunks == 1 and length == 13 and struct.unpack(">IIBBBBB", data) == (width, height, 8, 6, 0, 0, 0), "png_format")
        elif kind == b"sRGB":
            require(not srgb and not idat_seen and length == 1 and data[0] <= 3, "png_format")
            srgb = True
        elif kind == b"IDAT":
            require(not idat_ended and srgb, "png_shape")
            idat_seen = True
            compressed.extend(data)
            require(len(compressed) <= MAX_PNG, "png_compression")
        elif kind == b"IEND":
            require(length == 0 and idat_seen and at == len(raw), "png_shape")
            ended = True
        else:
            if idat_seen:
                idat_ended = True
            # Reject alternate color/animation interpretations and unknown critical chunks.
            require(kind not in (b"iCCP", b"acTL", b"fcTL", b"fdAT", b"tRNS", b"PLTE") and kind[0] & 32, "png_format")
    require(ended and srgb and idat_seen, "png_shape")
    size = height * (1 + width * 4)
    try:
        decoder = zlib.decompressobj()
        scan = decoder.decompress(compressed, size + 1)
        require(len(scan) == size and decoder.eof and not decoder.unconsumed_tail and not decoder.unused_data, "png_compression")
    except zlib.error:
        raise ValueError("png_compression") from None
    stride = width * 4
    pixels = bytearray(height * stride)
    for y in range(height):
        mode = scan[y * (stride + 1)]
        require(mode <= 4, "png_format")
        source = y * (stride + 1) + 1
        base = y * stride
        if mode == 0:
            pixels[base:base + stride] = scan[source:source + stride]
            continue
        for x in range(stride):
            left = pixels[base + x - 4] if x >= 4 else 0
            above = pixels[base + x - stride] if y else 0
            upper_left = pixels[base + x - stride - 4] if y and x >= 4 else 0
            predictor = left if mode == 1 else above if mode == 2 else (left + above) // 2 if mode == 3 else paeth(left, above, upper_left)
            pixels[base + x] = (scan[source + x] + predictor) & 255
    return bytes(pixels)


def canvas_pixels(path, metrics):
    raw = read_bytes(path, MAX_PNG, "png_file")
    require(len(raw) == metrics["png_bytes"] and hashlib.sha256(raw).hexdigest() == metrics["png_sha256"], "png_hash")
    pixels = decode_png(raw, metrics["width"], metrics["height"])
    require(hashlib.sha256(pixels).hexdigest() == metrics["decoded_rgba_sha256"], "decoded_hash")
    channels = [pixels[i::4] for i in range(4)]
    require([min(c) for c in channels] == metrics["rgba_min"] and [max(c) for c in channels] == metrics["rgba_max"]
            and [sum(c) for c in channels] == metrics["rgba_sum"]
            and sum(a != 255 for a in channels[3]) == metrics["alpha_not_opaque_pixels"], "metrics_mismatch")
    return pixels


def stroke_points(points, target, canvas, radius):
    require(isinstance(points, list) and len(points) == 6, "stroke_shape")
    uint(radius, 64, 1, "stroke_shape")
    out = []
    client = target["client_rect"]
    require(all(type(p) is int and 1000 <= p <= 999000 for p in points)
            and points[2] - points[0] == 1000 and points[4] - points[0] == 2000
            and points[1] == points[3] == points[5], "stroke_shape")
    for i in range(0, 6, 2):
        x = uint(points[i], 1000000, reason="stroke_shape")
        y = uint(points[i + 1], 1000000, reason="stroke_shape")
        # RemoteViewport.host floors physical coordinates. The fixed radius is
        # a conservative localization envelope, including viewport quantization;
        # it is not a measured brush size or an exact input-coordinate claim.
        x = client["x"] + x * client["width"] // 1000000 - canvas["x"]
        y = client["y"] + y * client["height"] // 1000000 - canvas["y"]
        require(radius <= x < canvas["width"] - radius and radius <= y < canvas["height"] - radius, "stroke_shape")
        out.append((x, y))
    return out


def near_segment(x, y, a, b, radius):
    dx, dy = b[0] - a[0], b[1] - a[1]
    length = dx * dx + dy * dy
    if length == 0:
        return (x - a[0]) ** 2 + (y - a[1]) ** 2 <= radius * radius
    projection = max(0, min(length, (x - a[0]) * dx + (y - a[1]) * dy))
    # Integer arithmetic avoids a floating tolerance expanding the accepted ROI.
    return ((x - a[0]) * length - projection * dx) ** 2 + ((y - a[1]) * length - projection * dy) ** 2 <= radius * radius * length * length


def cursor_samples(samples, canvas):
    require(isinstance(samples, list) and len(samples) == 4, "cursor_shape")
    prior = None
    for sample in samples:
        keys(sample, ("before", "after"), reason="cursor_shape")
        for point in (sample["before"], sample["after"]):
            keys(point, ("x", "y"), reason="cursor_shape")
            require(all(type(point[k]) is int and -(1 << 31) <= point[k] < (1 << 31) for k in ("x", "y")), "cursor_shape")
            require(prior is None or same(point, prior), "cursor_changed")
            require(not (canvas["x"] <= point["x"] < canvas["x"] + canvas["width"]
                         and canvas["y"] <= point["y"] < canvas["y"] + canvas["height"]), "cursor_inside_canvas")
            prior = point


def validate(value):
    json_shape(value)
    keys(value, TOP_KEYS, reason="envelope")
    require(type(value["schema"]) is int and value["schema"] == 1, "envelope")
    target, image, canvas = value["target"], value["image"], rect(value["canvas_rect"])
    validate_target(target)
    validate_image(image)
    require(inside(canvas, target["client_rect"]) and canvas["width"] <= 4096 and canvas["height"] <= 4096
            and canvas["width"] * canvas["height"] <= MAX_PIXELS, "canvas")
    expected = value["paint_frame"]
    _, previous_end = witness(expected, None, canvas)
    require(isinstance(value["baseline"], list) and len(value["baseline"]) == 2
            and isinstance(value["after"], list) and len(value["after"]) == 2
            and isinstance(value["canvas_files"], list) and len(value["canvas_files"]) == 4, "envelope")
    phone_start, phone_end = phone_receipts(value["phone"], value["native_phone"], target)
    metrics, frames = [], value["baseline"] + value["after"]
    for index, frame in enumerate(frames):
        if index == 2:
            require(previous_end < phone_start, "clock_order")
            previous_end = phone_end
        previous_end, m = native_frame(frame, target, image, expected, canvas, previous_end)
        metrics.append(m)
    require(len({r["sample_id"] for r in frames}) == 4
            and len({r["helper_sha256"] for r in frames}) == 1
            and all(same(r.get("installed_package_evidence"), frames[0].get("installed_package_evidence")) for r in frames), "native_identity")
    cursor_samples(value["cursor_samples"], canvas)
    require("stroke_ppm" not in value["phone"] or same(value["phone"]["stroke_ppm"], value["stroke_ppm"]), "phone_binding")
    points = stroke_points(value["stroke_ppm"], target, canvas, value["stroke_radius_pixels"])
    images = [canvas_pixels(path, m) for path, m in zip(value["canvas_files"], metrics)]
    require(images[0] == images[1], "baseline_unstable")
    require(images[2] == images[3], "after_unstable")
    require(images[0] == b"\xff" * len(images[0]), "baseline_not_white")
    require(images[0] != images[2], "no_rgb_change")
    changed, low_x, low_y, high_x, high_y = 0, canvas["width"], canvas["height"], 0, 0
    for pixel in range(canvas["width"] * canvas["height"]):
        at = pixel * 4
        if images[0][at:at + 3] == images[2][at:at + 3]:
            continue
        x, y = pixel % canvas["width"], pixel // canvas["width"]
        require(any(near_segment(x, y, a, b, value["stroke_radius_pixels"]) for a, b in zip(points, points[1:])), "outside_stroke_region")
        changed += 1
        low_x, low_y, high_x, high_y = min(low_x, x), min(low_y, y), max(high_x, x), max(high_y, y)
    require(changed > 0, "no_rgb_change")
    # Retain only verified closed fields so the parent can dispose private PNGs
    # and input receipts without losing the native sample/clock/hash bindings.
    samples = []
    for index, frame in enumerate(frames):
        m = metrics[index]
        samples.append({
            "stage": "baseline" if index < 2 else "after", "sample_id": frame["sample_id"], "capture_session_id": frame["capture_session_id"],
            "png_sha256": m["png_sha256"], "decoded_rgba_sha256": m["decoded_rgba_sha256"],
            "capture_monotonic_timestamp_ns": frame["canvas_frame"]["source_identity"]["monotonic_timestamp_ns"],
            "before_qpc_100ns": {"start": frame["before"]["observedStartQpc100ns"], "end": frame["before"]["observedEndQpc100ns"]},
            "after_qpc_100ns": {"start": frame["after"]["observedStartQpc100ns"], "end": frame["after"]["observedEndQpc100ns"]},
            "rgba_min": list(m["rgba_min"]), "rgba_max": list(m["rgba_max"]), "rgba_sum": list(m["rgba_sum"]), "alpha_not_opaque_pixels": 0,
        })
    retained = {
        "target_identity": {k: target[k] for k in ("window", "process_id", "thread_id", "process_created")},
        "canvas_rect_host": dict(canvas), "source_client_rect": dict(target["client_rect"]), "dpi": target["dpi"],
        "settings_digest": expected["settingsDigest"], "canvas_runtime_id_hash": expected["canvasRuntimeIdHash"],
        "initial_observation_qpc_100ns": {"start": expected["observedStartQpc100ns"], "end": expected["observedEndQpc100ns"]},
        "phone_call_qpc_100ns": {"start": phone_start, "end": phone_end}, "samples": samples,
    }
    return {"canvas_pixels": canvas["width"] * canvas["height"], "changed_rgb_pixels": changed,
            "changed_bbox_canvas": {"x": low_x, "y": low_y, "width": high_x - low_x + 1, "height": high_y - low_y + 1},
            "stroke_radius_pixels": value["stroke_radius_pixels"], "outside_stroke_changed_pixels": 0, "retained_proof": retained}


def report(value):
    result = {"schema": 1, "operation": "owned-paint-phone-visible-canvas", "status": "rejected", "rejection_reason": "unclassified",
              "accepted": False, "editor_effect": False, "visible_canvas_change": False, "native_receipt_correlation_only": True,
              "retained_parent_authority_required": True, "physical_pen_fidelity": False, "latency_acceptance": False,
              "workbench_ui_acceptance": False, "native_frame_count": 0, "pixel_evidence": None}
    try:
        pixels = validate(value)
    except (ValueError, TypeError, KeyError, OverflowError, OSError, RecursionError) as error:
        label = error.args[0] if len(error.args) == 1 and isinstance(error.args[0], str) else None
        result["rejection_reason"] = label if label in REASONS else "unclassified"
    else:
        result.update(status="passed", rejection_reason="none", accepted=True, editor_effect=True,
                      visible_canvas_change=True, native_frame_count=4, pixel_evidence=pixels)
    return result


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def nonfinite(_):
    raise ValueError("nonfinite_json")


def main(arguments):
    try:
        require(len(arguments) == 2 and arguments[0] == "--input", "arguments")
        raw = read_bytes(arguments[1], MAX_INPUT, "receipt_file")
        value = json.loads(raw.decode("utf-8-sig"), object_pairs_hook=unique, parse_constant=nonfinite)
        result = report(value)
    except (ValueError, OSError, TypeError, KeyError, OverflowError, RecursionError) as error:
        result = report(None)
        label = error.args[0] if len(error.args) == 1 and isinstance(error.args[0], str) else None
        result["rejection_reason"] = label if label in REASONS else "unclassified"
    print(json.dumps(result, separators=(",", ":"), allow_nan=False))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
