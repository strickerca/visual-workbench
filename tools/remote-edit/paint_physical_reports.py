"""Owner-physical Paint compatibility rows (B9 M6/M7).

Correlates the retained owner-physical Paint blocks: each block's native WGC
baseline/after canvas samples (mouse parked outside the canvas), its settled
remote receipt and its pulled phone journal, already validated by
physical_gesture_reports.py. It emits per-tool (paint.brush / paint.eraser)
"paint-compatibility-observations" documents for the exact Paint package,
executable hash and settings digest whose rows satisfy
RemoteToolCompatibility.validFor.

Results are text-only. Each tested row's evidenceSha256 is the SHA-256 of a
canonical evidence record retained in this report (journal, receipt and image
hashes plus measurements). Images stay under the caller's private root and are
disposed by it. Tilt stays NotTested (no calibration is wired) and Shortcuts
are left to T4.08.
"""
import hashlib
import json
import math
import re
import statistics
import sys
from pathlib import Path

import paint_phone_reports as paint
import physical_gesture_reports as gesture
from paint_phone_reports import near_segment, require, same, uint, keys, digest

ASPECTS = ("Drawing", "Pressure", "Hover", "Tilt", "InvertedEraser", "BarrelButton", "Shortcuts")
PAINT_BLOCKS = ("paint-pressure", "paint-hover", "paint-barrel", "paint-eraser", "paint-eraser-tool")
TOP_KEYS = "schema target image paint_frame eraser_frame canvas_rect stroke_radius_pixels device_model blocks".split()
BLOCK_KEYS = "block baseline after phone native_phone canvas_files cursor_samples".split()
PHONE_KEYS = ("schema status block native_receipt_path native_receipt_sha256 contract_sha256 journal_path journal_sha256 "
              "actual_remote_retirement retained_parent_returned receiver_observed editor_effect local_history_may_begin "
              "physical_owner_gesture software_generated_input latency_acceptance started_qpc_100ns completed_qpc_100ns").split()
MODEL = re.compile(r"[A-Za-z0-9_-]{1,64}\Z")
ERASER_TOOL_NAME = "Using Eraser tool on Canvas"
MIN_PRESSURE_POINTS = 20
SUPPORTED_R = 0.8
SUPPORTED_WIDTH_SPAN = 2.0
SUPPORTED_DARKNESS_SPAN = 20.0
UNSUPPORTED_CV = 0.05
UNSUPPORTED_PRESSURE_SPAN = 0.5
COLOUR_DISTANCE = 60.0
REASONS = frozenset(paint.REASONS | gesture.REASONS | set(
    "block_shape block_census eraser_witness device_model phone_physical journal_binding cursor_pairs chain_mismatch".split()))


def witness_eraser(value, brush, canvas):
    keys(value, paint.WITNESS_KEYS, reason="eraser_witness")
    for key in ("settingsDigest", "canvasRuntimeIdHash"):
        digest(value[key], "eraser_witness")
    require(value["canvasToolName"] == ERASER_TOOL_NAME and same(value["canvasRectHost"], canvas)
            and value["canvasRuntimeIdHash"] == brush["canvasRuntimeIdHash"], "eraser_witness")
    start = uint(value["observedStartQpc100ns"], paint.I64 // 100, 1, "eraser_witness")
    end = uint(value["observedEndQpc100ns"], paint.I64 // 100, 1, "eraser_witness")
    require(start < end, "eraser_witness")


def phone_physical(phone, native, block):
    keys(phone, PHONE_KEYS, reason="phone_physical")
    require(type(phone["schema"]) is int and phone["schema"] == 1 and phone["status"] == "phone-physical-correlated"
            and phone["block"] == block, "phone_physical")
    for key in ("native_receipt_sha256", "contract_sha256", "journal_sha256"):
        digest(phone[key], "phone_physical")
    for key in ("native_receipt_path", "journal_path"):
        require(isinstance(phone[key], str) and 0 < len(phone[key]) <= 32768, "phone_physical")
    for key in ("actual_remote_retirement", "retained_parent_returned", "local_history_may_begin", "physical_owner_gesture"):
        require(phone[key] is True, "phone_physical")
    for key in ("receiver_observed", "editor_effect", "software_generated_input", "latency_acceptance"):
        require(phone[key] is False, "phone_physical")
    require(isinstance(native, dict) and type(native.get("schema")) is int and native["schema"] == 1
            and native.get("kind") == "normal-native-remote-usb-integration" and native.get("status") == "passed"
            and native.get("model") == "SM-S918U" and type(native.get("owner_user")) is int and native["owner_user"] == 0
            and native.get("controller_input_scenario") == "owner-physical" and native.get("physical_owner_block") == block, "phone_physical")
    require(isinstance(native.get("run_id"), str) and re.fullmatch(r"[a-f0-9]{32}", native["run_id"])
            and native.get("retained_input_nonce") == native["run_id"]
            and native.get("owned_editor_contract_sha256") == phone["contract_sha256"]
            and native.get("physical_journal_sha256") == phone["journal_sha256"]
            and native.get("physical_journal_retained") == f"physical-journal-{native['run_id']}.json", "phone_physical")
    for key in ("controller_input_requested", "input_granted", "input_attempted", "input_dispatch_completed", "physical_owner_gesture",
                "physical_records_validated", "phone_render_records_validated", "input_retirement_confirmed", "host_actual_retirement",
                "phone_actual_retirement", "process_cleanup", "owned_packages_cleanup", "private_work_disposed",
                "external_editor_destination_retained", "profiles_preserved"):
        require(native.get(key) is True, "phone_physical")
    for key in ("software_generated_input", "editor_effect", "latency_acceptance", "controller_input_records_validated",
                "editor_phone_records_validated", "held_lifecycle_records_validated"):
        require(native.get(key) is False, "phone_physical")
    for key in ("failure_phase", "failure_error_type", "failure_error_code", "host_grant_failure", "host_first_run_failure", "host_first_carrier_failure"):
        require(native.get(key) is None, "phone_physical")
    validation = native.get("physical_validation")
    require(isinstance(validation, dict) and validation.get("journal_sha256") == phone["journal_sha256"]
            and validation.get("physical_owner_gesture") is True and validation.get("block") == block, "phone_physical")
    start = uint(phone["started_qpc_100ns"], paint.I64 // 100, 1, "clock_order")
    end = uint(phone["completed_qpc_100ns"], paint.I64 // 100, 1, "clock_order")
    require(start < end, "clock_order")
    return start, end


def load_journal(phone, native):
    raw = paint.read_bytes(phone["journal_path"], gesture.JOURNAL_LIMIT, "journal_binding")
    require(hashlib.sha256(raw).hexdigest() == phone["journal_sha256"], "journal_binding")
    journal = gesture.parse(raw)
    gesture.validate_journal(journal, raw)
    require(journal["block"] == phone["block"], "journal_binding")
    # The retained receipt's phone record must still bind these exact bytes.
    gesture.validate_record(native.get("physical_records"), native.get("phone_render_records"), journal, raw)
    return journal, raw


def cursor_pairs(samples, canvas):
    require(isinstance(samples, list) and len(samples) == 4, "cursor_shape")
    points = []
    for sample in samples:
        keys(sample, ("before", "after"), reason="cursor_shape")
        for point in (sample["before"], sample["after"]):
            keys(point, ("x", "y"), reason="cursor_shape")
            require(all(type(point[k]) is int and -(1 << 31) <= point[k] < (1 << 31) for k in ("x", "y")), "cursor_shape")
            require(not (canvas["x"] <= point["x"] < canvas["x"] + canvas["width"]
                         and canvas["y"] <= point["y"] < canvas["y"] + canvas["height"]), "cursor_inside_canvas")
            points.append(point)
    # The parked pointer is stable within each baseline pair and each after pair;
    # the owner may use the Paint toolbar between them (tool switch), never the canvas.
    require(all(same(points[0], p) for p in points[:4]) and all(same(points[4], p) for p in points[4:]), "cursor_pairs")


def strokes_in_canvas(journal, canvas):
    """Contact strokes from the journal, mapped to canvas-relative coordinates."""
    out, current = [], None
    for s in journal["samples"]:
        point = (s["host_x"] - canvas["x"], s["host_y"] - canvas["y"], s["raw_pressure"], s["phase"])
        if s["phase"] == "Down":
            current = [point]
        elif s["phase"] in ("Move", "Up") and current is not None:
            current.append(point)
            if s["phase"] == "Up":
                out.append(current)
                current = None
    return out


def hover_points(journal, canvas):
    return [(s["host_x"] - canvas["x"], s["host_y"] - canvas["y"]) for s in journal["samples"] if s["phase"] == "Hover"]


def changed_pixels(before, after, width, height):
    out = []
    for pixel in range(width * height):
        at = pixel * 4
        if before[at:at + 3] != after[at:at + 3]:
            out.append((pixel % width, pixel // width))
    return out


def inside_region(points, segments, radius):
    for x, y in points:
        require(any(near_segment(x, y, a, b, radius) for a, b in segments), "outside_stroke_region")


def segments_of(strokes):
    out = []
    for stroke in strokes:
        xy = [(p[0], p[1]) for p in stroke]
        out.extend(zip(xy, xy[1:]) if len(xy) > 1 else [(xy[0], xy[0])])
    return out


def luminance(pixels, x, y, width):
    at = (y * width + x) * 4
    r, g, b = pixels[at], pixels[at + 1], pixels[at + 2]
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def mean_colour(pixels, points, width):
    if not points:
        return None
    total = [0, 0, 0]
    for x, y in points:
        at = (y * width + x) * 4
        for c in range(3):
            total[c] += pixels[at + c]
    return [v / len(points) for v in total]


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, max(0, int(round(fraction * (len(ordered) - 1)))))]


def measure_pressure(strokes, changed, after, canvas, radius):
    """Local ink width/darkness around spaced interior samples of each stroke."""
    width_px, height_px = canvas["width"], canvas["height"]
    mask = set(changed)
    disk = [(dx, dy) for dy in range(-radius, radius + 1) for dx in range(-radius, radius + 1) if dx * dx + dy * dy <= radius * radius]
    points = []
    for index, stroke in enumerate(strokes):
        others = [(p[0], p[1]) for j, other in enumerate(strokes) if j != index for p in other]
        last = None
        for position, (x, y, pressure, phase) in enumerate(stroke):
            if phase != "Move" or position < 3 or position > len(stroke) - 4:
                continue
            if not (radius <= x < width_px - radius and radius <= y < height_px - radius):
                continue
            if last is not None and (x - last[0]) ** 2 + (y - last[1]) ** 2 < (radius // 2) ** 2:
                continue
            if any((x - ox) ** 2 + (y - oy) ** 2 <= (2 * radius) ** 2 for ox, oy in others):
                continue
            inked = [(x + dx, y + dy) for dx, dy in disk if (x + dx, y + dy) in mask]
            darkness = statistics.fmean(255 - luminance(after, px, py, width_px) for px, py in inked) if inked else 0.0
            points.append({"pressure": pressure, "width": len(inked) / (2 * radius), "darkness": darkness})
            last = (x, y)
    return points


def pressure_result(points):
    summary = {"points": len(points), "width_r": None, "darkness_r": None, "width_span": None, "darkness_span": None,
               "width_cv": None, "darkness_cv": None, "pressure_span": None}
    if len(points) < MIN_PRESSURE_POINTS:
        return "Inconclusive", summary
    pressures = [p["pressure"] for p in points]
    widths = [p["width"] for p in points]
    darkness = [p["darkness"] for p in points]
    summary.update(width_r=gesture.pearson(list(zip(pressures, widths))), darkness_r=gesture.pearson(list(zip(pressures, darkness))),
                   width_span=percentile(widths, 0.9) - percentile(widths, 0.1),
                   darkness_span=percentile(darkness, 0.9) - percentile(darkness, 0.1),
                   pressure_span=max(pressures) - min(pressures))
    mean_width, mean_dark = statistics.fmean(widths), statistics.fmean(darkness)
    summary["width_cv"] = statistics.pstdev(widths) / mean_width if mean_width > 0 else None
    summary["darkness_cv"] = statistics.pstdev(darkness) / mean_dark if mean_dark > 0 else None
    width_ok = summary["width_r"] is not None and summary["width_r"] >= SUPPORTED_R and summary["width_span"] >= SUPPORTED_WIDTH_SPAN
    dark_ok = summary["darkness_r"] is not None and summary["darkness_r"] >= SUPPORTED_R and summary["darkness_span"] >= SUPPORTED_DARKNESS_SPAN
    if width_ok or dark_ok:
        return "Supported", summary
    flat = (summary["width_cv"] is not None and summary["width_cv"] <= UNSUPPORTED_CV
            and summary["darkness_cv"] is not None and summary["darkness_cv"] <= UNSUPPORTED_CV)
    if flat and summary["pressure_span"] >= UNSUPPORTED_PRESSURE_SPAN:
        return "Unsupported", summary
    return "Inconclusive", summary


def canonical_sha(record):
    return hashlib.sha256(json.dumps(record, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def row(aspect, result, evidence_sha, device_model):
    if result == "NotTested":
        return {"aspect": aspect, "result": "NotTested", "method": "NotPerformed", "inputApi": "Unknown",
                "evidenceSha256": None, "deviceModel": None}
    return {"aspect": aspect, "result": result, "method": "PhysicalOwnerGesture", "inputApi": "WindowsPointerInput",
            "evidenceSha256": evidence_sha, "deviceModel": device_model}


def compatibility_document(image, tool, settings, results, device_model):
    """One per-tool interchange document (NativeRemoteCompatibility.decode)."""
    identity = {"editor": "Paint", "executableName": image["executableName"], "executableBlake3": image["executableBlake3"],
                "executableBytes": image["executableBytes"], "fileVersion": image["fileVersion"],
                "packageFullName": image["packageFullName"], "packageVersion": image["packageVersion"],
                "shortcutProfileDigest": None}
    observations = []
    for aspect in ASPECTS:
        result, evidence_sha = results.get(aspect, ("NotTested", None))
        observations.append(row(aspect, result, evidence_sha, device_model))
    return {"schema": 1, "kind": "paint-compatibility-observations", "identity": identity, "toolId": tool,
            "settingsDigest": settings, "observations": observations}


def document_bytes(document):
    return json.dumps(document, separators=(",", ":"), allow_nan=False).encode()


def validate(value):
    paint.json_shape(value)
    keys(value, TOP_KEYS, reason="envelope")
    require(type(value["schema"]) is int and value["schema"] == 1, "envelope")
    target, image, canvas = value["target"], value["image"], paint.rect(value["canvas_rect"])
    paint.validate_target(target)
    paint.validate_image(image)
    require(paint.inside(canvas, target["client_rect"]) and canvas["width"] <= 4096 and canvas["height"] <= 4096
            and canvas["width"] * canvas["height"] <= paint.MAX_PIXELS, "canvas")
    brush = value["paint_frame"]
    _, previous_end = paint.witness(brush, None, canvas)
    radius = uint(value["stroke_radius_pixels"], 64, 1, "stroke_shape")
    device = value["device_model"]
    require(isinstance(device, str) and MODEL.fullmatch(device), "device_model")
    blocks = value["blocks"]
    require(isinstance(blocks, list) and 1 <= len(blocks) <= len(PAINT_BLOCKS), "block_census")
    names = []
    for block in blocks:
        keys(block, BLOCK_KEYS, reason="block_shape")
        require(block["block"] in PAINT_BLOCKS and block["block"] not in names, "block_census")
        names.append(block["block"])
    eraser = value["eraser_frame"]
    require((eraser is not None) == ("paint-eraser-tool" in names), "eraser_witness")
    if eraser is not None:
        witness_eraser(eraser, brush, canvas)

    analyses, helpers, packages, previous_after = {}, set(), [], None
    for index, block in enumerate(blocks):
        name = block["block"]
        require(isinstance(block["baseline"], list) and len(block["baseline"]) == 2 and isinstance(block["after"], list)
                and len(block["after"]) == 2 and isinstance(block["canvas_files"], list) and len(block["canvas_files"]) == 4, "block_shape")
        phone_start, phone_end = phone_physical(block["phone"], block["native_phone"], name)
        frames, metrics = block["baseline"] + block["after"], []
        for position, frame in enumerate(frames):
            if position == 2:
                require(previous_end < phone_start, "clock_order")
                previous_end = phone_end
            previous_end, m = paint.native_frame(frame, target, image, brush, canvas, previous_end)
            metrics.append(m)
            helpers.add(frame["helper_sha256"])
            packages.append(frame.get("installed_package_evidence"))
        cursor_pairs(block["cursor_samples"], canvas)
        images = [paint.canvas_pixels(path, m) for path, m in zip(block["canvas_files"], metrics)]
        require(images[0] == images[1], "baseline_unstable")
        require(images[2] == images[3], "after_unstable")
        if index == 0:
            require(images[0] == b"\xff" * len(images[0]), "baseline_not_white")
        else:
            require(images[0] == previous_after, "chain_mismatch")
        previous_after = images[2]
        journal, raw = load_journal(block["phone"], block["native_phone"])
        changed = changed_pixels(images[0], images[2], canvas["width"], canvas["height"])
        record = {"block": name, "journal_sha256": hashlib.sha256(raw).hexdigest(),
                  "native_receipt_sha256": block["phone"]["native_receipt_sha256"],
                  "sample_ids": [f["sample_id"] for f in frames],
                  "png_sha256": [m["png_sha256"] for m in metrics], "decoded_rgba_sha256": [m["decoded_rgba_sha256"] for m in metrics],
                  "changed_rgb_pixels": len(changed), "canvas_rect_host": dict(canvas), "stroke_radius_pixels": radius}
        strokes = strokes_in_canvas(journal, canvas)
        if name == "paint-hover":
            hovers = hover_points(journal, canvas)
            if changed:
                inside_region(changed, [(p, p) for p in hovers] or [((-radius * 4, -radius * 4),) * 2], radius)
            record["hover_samples_in_canvas"] = sum(0 <= x < canvas["width"] and 0 <= y < canvas["height"] for x, y in hovers)
        else:
            inside_region(changed, segments_of(strokes), radius)
            record["contact_strokes"] = len(strokes)
        analyses[name] = {"changed": changed, "after": images[2], "before": images[0], "strokes": strokes, "journal": journal, "record": record}
    sample_ids = [f["sample_id"] for block in blocks for f in block["baseline"] + block["after"]]
    require(len(helpers) == 1 and len(set(sample_ids)) == len(sample_ids)
            and all(same(p, packages[0]) for p in packages), "native_identity")
    return build_results(value, analyses, radius)


def build_results(value, analyses, radius):
    canvas, image, device = value["canvas_rect"], value["image"], value["device_model"]
    width = canvas["width"]
    evidence = {}

    def tested(aspect, tool, result, record, **extra):
        payload = dict(record, aspect=aspect, tool=tool, result=result, **extra)
        sha = canonical_sha(payload)
        evidence[sha] = payload
        return result, sha

    brush, eraser = {}, {}
    primary = None
    for name in ("paint-pressure", "paint-barrel"):
        if name in analyses:
            a = analyses[name]
            result = "Supported" if a["changed"] else "Unsupported"
            brush["Drawing"] = tested("Drawing", "paint.brush", result, a["record"])
            break
    if "paint-pressure" in analyses:
        a = analyses["paint-pressure"]
        primary = mean_colour(a["after"], a["changed"], width)
        points = measure_pressure(a["strokes"], a["changed"], a["after"], canvas, radius)
        result, summary = pressure_result(points)
        brush["Pressure"] = tested("Pressure", "paint.brush", result, a["record"], measurement=summary)
    if "paint-hover" in analyses:
        a = analyses["paint-hover"]
        inside = a["record"]["hover_samples_in_canvas"]
        result = "Inconclusive" if not inside else ("Supported" if not a["changed"] else "Unsupported")
        brush["Hover"] = tested("Hover", "paint.brush", result, a["record"])
    if "paint-barrel" in analyses:
        a = analyses["paint-barrel"]
        colour = mean_colour(a["after"], a["changed"], width)
        if not a["changed"] or primary is None:
            result = "Inconclusive"
        else:
            distance = math.dist(colour, primary)
            result = "Supported" if distance > COLOUR_DISTANCE else "Unsupported"
        brush["BarrelButton"] = tested("BarrelButton", "paint.brush", result, a["record"],
                                       barrel_mean_rgb=[round(c, 3) for c in colour] if colour else None,
                                       primary_mean_rgb=[round(c, 3) for c in primary] if primary else None)
    if "paint-eraser" in analyses:
        a = analyses["paint-eraser"]
        if any(s["pen_flags"] & gesture.PEN_FLAG_ERASER for s in a["journal"]["samples"]):
            erased = sum(all(a["after"][(y * width + x) * 4 + c] >= 245 for c in range(3)) for x, y in a["changed"])
            if not a["changed"]:
                result = "Inconclusive"
            else:
                result = "Supported" if erased >= 0.9 * len(a["changed"]) else "Unsupported"
            brush["InvertedEraser"] = tested("InvertedEraser", "paint.brush", result, a["record"], erased_pixels=erased)
    if "paint-eraser-tool" in analyses:
        a = analyses["paint-eraser-tool"]
        erased = sum(all(a["after"][(y * width + x) * 4 + c] >= 245 for c in range(3)) for x, y in a["changed"])
        if not a["changed"]:
            result = "Inconclusive"
        else:
            result = "Supported" if erased >= 0.9 * len(a["changed"]) else "Unsupported"
        eraser["Drawing"] = tested("Drawing", "paint.eraser", result, a["record"], erased_pixels=erased)
    documents = [compatibility_document(image, "paint.brush", value["paint_frame"]["settingsDigest"], brush, device)]
    if value["eraser_frame"] is not None:
        documents.append(compatibility_document(image, "paint.eraser", value["eraser_frame"]["settingsDigest"], eraser, device))
    editor_effect = any(a["changed"] for name, a in analyses.items() if name != "paint-hover")
    summaries = [{"block": name, "changed_rgb_pixels": len(a["changed"]), "journal_sha256": a["record"]["journal_sha256"]}
                 for name, a in analyses.items()]
    return {"documents": documents, "document_sha256": [hashlib.sha256(document_bytes(d)).hexdigest() for d in documents],
            "evidence": evidence, "blocks": summaries, "editor_effect": editor_effect}


def report(value):
    result = {"schema": 1, "operation": "owned-paint-physical-compatibility", "status": "rejected", "rejection_reason": "unclassified",
              "accepted": False, "editor_effect": False, "physical_owner_gesture": False, "latency_acceptance": False,
              "tilt_calibrated": False, "shortcuts_tested": False, "documents": [], "document_sha256": [], "evidence": {}, "blocks": []}
    try:
        validated = validate(value)
    except (ValueError, TypeError, KeyError, OverflowError, OSError, RecursionError) as error:
        label = error.args[0] if len(error.args) == 1 and isinstance(error.args[0], str) else None
        result["rejection_reason"] = label if label in REASONS else "unclassified"
    else:
        result.update(validated, status="passed", rejection_reason="none", accepted=True, physical_owner_gesture=True)
    return result


def main(arguments):
    try:
        require(len(arguments) == 2 and arguments[0] == "--input", "arguments")
        raw = paint.read_bytes(arguments[1], paint.MAX_INPUT * 8, "receipt_file")
        value = json.loads(raw.decode("utf-8-sig"), object_pairs_hook=paint.unique, parse_constant=paint.nonfinite)
        result = report(value)
    except (ValueError, OSError, TypeError, KeyError, OverflowError, RecursionError) as error:
        result = report(None)
        label = error.args[0] if len(error.args) == 1 and isinstance(error.args[0], str) else None
        result["rejection_reason"] = label if label in REASONS else "unclassified"
    print(json.dumps(result, separators=(",", ":"), allow_nan=False))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
