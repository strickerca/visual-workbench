"""Closed validator for the owner-physical remote pen route (B9 M3/M5).

Inputs are source-owned text receipts only: the phone physical-gesture journal
pulled by root, the phone status record, the three render views and, on the
owned harness route, the receiver counts and pen-history journal. The existing
synthetic validators stay unchanged; this route alone admits barrel/eraser pen
flags, and only for input every predicate classifies as PhysicalStylus.

What a pass proves (PHYSICAL_CLAIM): every admitted sample entered the
Activity window dispatch, every DOWN/UP carried a verified InputDispatcher
signature (API 30+), the device is a non-virtual stylus+touchscreen digitizer
(not SOURCE_STYLUS-only shell injection), no accessibility/generated-gesture
flag, zero synthetic dispatches, hardware-shaped axes (varying pressure in
every contact block, declared tilt/orientation/distance present and varying,
batched history), native ACK coverage and, on the harness route, receiver
correlation. What it does not prove: absence of a privileged injector that
crafts events matching all of these, a human hand on the pen, latency, tilt
calibration, or any editor effect.
"""
import collections
import hashlib
import importlib.util
import json
import math
import re
import statistics
import sys
from pathlib import Path

from controller_input_reports import validate_provenance
from integration_reports import UUID, integer, require, validate as validate_views

_ANALYZE = Path(__file__).resolve().parents[1] / "pen-inject" / "analyze.py"
_spec = importlib.util.spec_from_file_location("vw_pen_inject_analyze", _ANALYZE)
_analyze = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_analyze)
pearson = _analyze.pearson  # Reuse the T0.05 Pearson implementation unchanged.

JOURNAL_LIMIT = 6 * 1024 * 1024
RECORD_LIMIT = 65536
RECEIVER_LIMIT = 16 * 1024 * 1024
I64 = (1 << 63) - 1
U64 = (1 << 64) - 1
SHA = re.compile(r"[a-f0-9]{64}\Z")
RUN = re.compile(r"[a-f0-9]{32}\Z")
MODEL = re.compile(r"[A-Za-z0-9_-]{1,64}\Z")

BLOCKS = ("pressure", "hover", "barrel", "eraser",
          "paint-pressure", "paint-hover", "paint-barrel", "paint-eraser", "paint-eraser-tool")
HARNESS_BLOCKS = BLOCKS[:4]
HOVER_BLOCKS = ("hover", "paint-hover")
PHASES = ("Hover", "Down", "Move", "Up", "Leave")
ORIGINS = ("PhysicalStylus", "Injected", "Direct", "Unknown")
ROUTE = ("surface_touch,software_generated=false,physical_owner_gesture=true,window_delivered=true,"
         "synthetic_dispatches=0,ack_covered=true,latency_acceptance=false")
PHYSICAL_CLAIM = ("window_dispatched,dispatcher_signed_down_up,non_virtual_pen_digitizer_source,no_a11y_or_generated_flag,"
                  "zero_synthetic_dispatch,hardware_shaped_axes;not_proof_against_crafted_privileged_injection")
VERIFICATIONS = ("Verified", "Refused", "NotSigned", "Unavailable", "NotDelivered")

TOP_KEYS = frozenset(("schema kind run_id block target_strokes hover_only device_model sdk clock "
                      "timestamp_resolution_ns complete stop_reason synthetic_dispatches window_deliveries "
                      "admitted_strokes acknowledged_through echo input_devices events samples owner_pauses").split())
DEVICE_KEYS = frozenset("device_id sources declares_stylus virtual external ranges".split())
RANGE_KEYS = frozenset("axis source min max flat fuzz resolution".split())
EVENT_KEYS = frozenset(("event action device_id device_virtual declares_stylus external device_sources source tool_type flags "
                        "button_state action_button classification history_size event_time_ns window_delivered verification origin").split())
SAMPLE_KEYS = frozenset(("seq event phase host_x host_y sent_pressure raw_pressure raw_tilt raw_orientation "
                         "raw_distance tilt_x tilt_y pen_mask pen_flags sampled_ns historical origin").split())
RECORD_KEYS = frozenset("remote_physical_" + k for k in
                        "binding block strokes samples sequences owner ack echo witness journal route".split())
RECEIVER_KEYS = frozenset("message index count pointer_available pressure pen_flags performance_count pointer_flags pointer_id".split())
HISTORY_KEYS = frozenset("x y pressure tilt_x tilt_y rotation pen_flags pen_mask pointer_flags frame_id performance_count history_count".split())

# Android constants mirrored from RemotePhysicalGestureJournal.kt.
ACCESSIBILITY_FLAG = 0x800
GENERATED_GESTURE_FLAG = 0x8
SOURCE_STYLUS = 0x4002
PEN_DIGITIZER = 0x5002  # SOURCE_STYLUS|SOURCE_TOUCHSCREEN, as a built-in pen digitizer reports
AXIS_ORIENTATION, AXIS_DISTANCE, AXIS_TILT = 8, 24, 25
TOOL_STYLUS, TOOL_ERASER = 2, 4
BUTTON_STYLUS_PRIMARY = 0x20
# Windows pointer flags.
INRANGE, INCONTACT, DOWN_FLAG, UP_FLAG = 0x2, 0x4, 0x10000, 0x40000
PEN_FLAG_BARREL, PEN_FLAG_ERASER = 0x1, 0x4

PRESSURE_MIN_PAIRS = 200
PRESSURE_MIN_STROKES = 5
TRANSPORT_MIN_R = 0.95  # receiver == round(raw*1024): transport fidelity, not owner variation
PRESSURE_MIN_SPAN = 0.5
PRESSURE_MIN_STDEV = 0.1
CONTACT_MIN_PRESSURE_SPAN = 0.1  # every contact block: a constant-pressure injector is refused

REASONS = frozenset((
    "arguments receipt_file receipt_file_bytes duplicate_json_key nonfinite_json json_shape "
    "journal_shape journal_identity journal_incomplete journal_block synthetic_dispatch window_delivery "
    "device_census event_shape origin_mismatch non_physical_origin sample_shape sequence_census "
    "sample_event sample_order phase_transition sample_pressure sample_flags owner_pause "
    "stroke_census echo_coverage ack_coverage record_shape record_binding record_scope record_owner "
    "record_journal record_ack record_route software_route_refused render_census scope_shape "
    "scope_uuid owner_uuid integer_shape integer_bound codec_name timing_shape local_callback_order "
    "capability_shape fresh_owner_capture_scope background_and_reconnect_epochs scenario "
    "constant_pressure pressure_span pressure_pairs pressure_strokes transport_pressure_fidelity pressure_variance "
    "hardware_axes hardware_history run_binding "
    "hover_contact hover_not_observed barrel_not_observed eraser_not_observed block_flags "
    "receiver_counts receiver_mouse receiver_event_shape receiver_provenance_shape receiver_history "
    "receiver_missing_contact receiver_missing_hover receiver_order receiver_unmatched receiver_flags "
    "receiver_pressure receiver_hover receiver_mode unclassified").split())


def uint(value, maximum, reason, minimum=0):
    require(type(value) is int and minimum <= value <= maximum, reason)
    return value


def sint(value, low, high, reason):
    require(type(value) is int and low <= value <= high, reason)
    return value


def boolean(value, reason, nullable=False):
    require(type(value) is bool or (nullable and value is None), reason)
    return value


def finite(value, reason, nullable=False):
    if nullable and value is None:
        return None
    require(type(value) in (int, float) and math.isfinite(value), reason)
    return float(value)


def closed(value, expected, reason):
    require(isinstance(value, dict) and set(value) == expected, reason)


def classify(event, synthetic):
    """Mirror of classifyRemoteInput (RemotePhysicalGestureJournal.kt)."""
    if not event["window_delivered"]:
        return "Direct"
    if (event["device_id"] == -1 or event["device_virtual"] is True
            or event["flags"] & (ACCESSIBILITY_FLAG | GENERATED_GESTURE_FLAG) or event["verification"] == "Refused"):
        return "Injected"
    if synthetic != 0:
        return "Unknown"
    if event["device_virtual"] is None or not event["declares_stylus"]:
        return "Unknown"
    if event["tool_type"] not in (TOOL_STYLUS, TOOL_ERASER):
        return "Unknown"
    sources = event["device_sources"]
    if sources is None or sources & PEN_DIGITIZER != PEN_DIGITIZER or event["source"] & PEN_DIGITIZER != PEN_DIGITIZER:
        return "Unknown"
    if event["action"] & 0xFF in (0, 1):  # DOWN/UP are the only dispatcher-signed actions
        return "PhysicalStylus" if event["verification"] == "Verified" else "Unknown"
    return "PhysicalStylus"


def strokes(samples, hover_only):
    """Count contact strokes (Down..Up) or hover passes (Hover..Leave, no contact)."""
    count, contact, hovering, current, out = 0, False, False, [], []
    for s in samples:
        phase = s["phase"]
        if phase == "Down":
            contact, hovering, current = True, False, [s]
        elif phase == "Move":
            current.append(s)
        elif phase == "Up":
            if contact:
                current.append(s)
                if not hover_only:
                    count += 1
                    out.append(current)
            contact, current = False, []
        elif phase == "Hover":
            if not contact:
                hovering = True
        else:
            if hover_only and hovering and not contact:
                count += 1
            hovering, contact = False, False
    return count, out


def hardware_shape(events, samples, census, kind):
    """Axes a hardware pen digitizer produces and a constant injector does not.

    Returns the raw contact-pressure span (None for hover blocks)."""
    declared = {r["axis"] for e in events for r in census[e["device_id"]]["ranges"]}
    require(any(e["history_size"] > 0 for e in events), "hardware_history")
    contact = [s for s in samples if s["phase"] in ("Down", "Move", "Up")]
    hovers = [s for s in samples if s["phase"] == "Hover"]
    for axis, key in ((AXIS_TILT, "raw_tilt"), (AXIS_ORIENTATION, "raw_orientation")):
        if axis in declared:
            for rows in (contact, hovers if kind == "hover" else []):
                if rows:
                    values = [s[key] for s in rows]
                    require(None not in values and len(set(values)) > 1, "hardware_axes")
    if AXIS_DISTANCE in declared and len(hovers) >= 2:
        values = [s["raw_distance"] for s in hovers]
        require(None not in values and len(set(values)) > 1, "hardware_axes")
    if kind == "hover":
        require(len(hovers) >= 2, "hover_not_observed")
        return None
    require(any(s["historical"] for s in contact), "hardware_history")
    pressures = [s["raw_pressure"] for s in samples if s["phase"] in ("Down", "Move")]
    require(len(set(pressures)) > 1, "constant_pressure")
    span = max(pressures) - min(pressures)
    require(span >= CONTACT_MIN_PRESSURE_SPAN, "pressure_span")
    return span


def validate_journal(journal, raw):
    closed(journal, TOP_KEYS, "journal_shape")
    require(type(journal["schema"]) is int and journal["schema"] == 1
            and journal["kind"] == "remote-physical-gesture-journal", "journal_shape")
    require(isinstance(journal["run_id"], str) and RUN.fullmatch(journal["run_id"]), "journal_identity")
    block = journal["block"]
    require(block in BLOCKS, "journal_block")
    target = uint(journal["target_strokes"], 64, "journal_block", 1)
    hover_only = block in HOVER_BLOCKS
    require(journal["hover_only"] is hover_only, "journal_block")
    require(isinstance(journal["device_model"], str) and MODEL.fullmatch(journal["device_model"]), "journal_identity")
    uint(journal["sdk"], 99, "journal_identity", 29)
    require(journal["clock"] == "android_uptime_ns"
            and type(journal["timestamp_resolution_ns"]) is int
            and journal["timestamp_resolution_ns"] in (1, 1_000_000), "journal_shape")
    require(journal["complete"] is True and journal["stop_reason"] == "target_reached", "journal_incomplete")
    synthetic = uint(journal["synthetic_dispatches"], I64, "synthetic_dispatch")
    require(synthetic == 0, "synthetic_dispatch")
    deliveries = uint(journal["window_deliveries"], I64, "window_delivery")
    require(isinstance(journal["owner_pauses"], list), "journal_shape")
    for pause in journal["owner_pauses"]:
        closed(pause, {"reason", "count"}, "journal_shape")
    require(journal["owner_pauses"] == [], "owner_pause")

    devices = journal["input_devices"]
    require(isinstance(devices, list) and 1 <= len(devices) <= 16, "device_census")
    census = {}
    for d in devices:
        closed(d, DEVICE_KEYS, "device_census")
        identity = sint(d["device_id"], -(1 << 31), (1 << 31) - 1, "device_census")
        require(identity not in census, "device_census")
        sint(d["sources"], -(1 << 31), (1 << 31) - 1, "device_census")
        for key in ("declares_stylus", "virtual", "external"):
            boolean(d[key], "device_census")
        require(isinstance(d["ranges"], list) and len(d["ranges"]) <= 64, "device_census")
        for r in d["ranges"]:
            closed(r, RANGE_KEYS, "device_census")
            sint(r["axis"], 0, 63, "device_census")
            sint(r["source"], -(1 << 31), (1 << 31) - 1, "device_census")
            for key in ("min", "max", "flat", "fuzz", "resolution"):
                finite(r[key], "device_census")
        census[identity] = d

    events = journal["events"]
    require(isinstance(events, list) and 1 <= len(events) <= 6000, "event_shape")
    for index, e in enumerate(events):
        closed(e, EVENT_KEYS, "event_shape")
        require(type(e["event"]) is int and e["event"] == index, "event_shape")
        for key in ("action", "source", "flags", "button_state", "action_button", "classification"):
            sint(e[key], -(1 << 31), (1 << 31) - 1, "event_shape")
        sint(e["device_id"], -(1 << 31), (1 << 31) - 1, "event_shape")
        boolean(e["device_virtual"], "event_shape", nullable=True)
        boolean(e["external"], "event_shape", nullable=True)
        require(e["device_sources"] is None or (type(e["device_sources"]) is int and -(1 << 31) <= e["device_sources"] < (1 << 31)), "event_shape")
        require(e["verification"] in VERIFICATIONS and (e["verification"] == "NotDelivered") != e["window_delivered"], "event_shape")
        boolean(e["declares_stylus"], "event_shape")
        boolean(e["window_delivered"], "event_shape")
        uint(e["tool_type"], 255, "event_shape")
        uint(e["history_size"], 511, "event_shape")
        uint(e["event_time_ns"], I64, "event_shape")
        require(e["origin"] in ORIGINS, "event_shape")
        require(classify(e, synthetic) == e["origin"], "origin_mismatch")
    require(all(e["origin"] == "PhysicalStylus" for e in events), "non_physical_origin")
    require(deliveries >= len(events), "window_delivery")
    for e in events:
        device = census.get(e["device_id"])
        require(device is not None and device["declares_stylus"] is True and device["virtual"] is False
                and device["virtual"] is e["device_virtual"] and device["external"] is e["external"]
                and device["sources"] == e["device_sources"] and device["sources"] & PEN_DIGITIZER == PEN_DIGITIZER, "device_census")

    samples = journal["samples"]
    require(isinstance(samples, list) and 1 <= len(samples) <= 12000, "sample_shape")
    contact, previous_event, previous_time, historical_rows = False, -1, 0, {}
    for index, s in enumerate(samples):
        closed(s, SAMPLE_KEYS, "sample_shape")
        require(type(s["seq"]) is int and s["seq"] == index + 1, "sequence_census")
        uint(s["event"], len(events) - 1, "sample_event")
        require(s["event"] >= previous_event, "sample_order")
        previous_event = s["event"]
        event = events[s["event"]]
        require(s["phase"] in PHASES, "sample_shape")
        sint(s["host_x"], -(1 << 31), (1 << 31) - 1, "sample_shape")
        sint(s["host_y"], -(1 << 31), (1 << 31) - 1, "sample_shape")
        sent = uint(s["sent_pressure"], 1024, "sample_pressure")
        raw_pressure = finite(s["raw_pressure"], "sample_pressure")
        require(raw_pressure >= 0, "sample_pressure")
        expected = math.floor(min(raw_pressure, 1.0) * 1024 + 0.5)
        require(abs(sent - expected) <= 1, "sample_pressure")
        for key in ("raw_tilt", "raw_orientation", "raw_distance"):
            finite(s[key], "sample_shape", nullable=True)
        sint(s["tilt_x"], -90, 90, "sample_shape")
        sint(s["tilt_y"], -90, 90, "sample_shape")
        mask = uint(s["pen_mask"], 15, "sample_shape")
        require(mask & 1 == 1, "sample_shape")
        flags = uint(s["pen_flags"], 7, "sample_flags")
        require(flags & 2 == 0, "sample_flags")
        require(bool(flags & PEN_FLAG_BARREL) == bool(event["button_state"] & BUTTON_STYLUS_PRIMARY)
                and bool(flags & PEN_FLAG_ERASER) == (event["tool_type"] == TOOL_ERASER), "sample_flags")
        sampled = uint(s["sampled_ns"], I64, "sample_shape", 1)
        require(sampled >= previous_time, "sample_order")
        previous_time = sampled
        boolean(s["historical"], "sample_shape")
        require(not s["historical"] or s["phase"] == "Move", "sample_shape")
        if s["historical"]:
            historical_rows[s["event"]] = historical_rows.get(s["event"], 0) + 1
            require(historical_rows[s["event"]] <= event["history_size"], "sample_shape")
        require(s["origin"] == event["origin"] == "PhysicalStylus", "non_physical_origin")
        phase = s["phase"]
        if phase == "Down":
            require(not contact, "phase_transition")
            contact = True
        elif phase in ("Move", "Up"):
            require(contact, "phase_transition")
            contact = phase == "Move"
        else:
            require(not contact, "phase_transition")

    count, contact_strokes = strokes(samples, hover_only)
    require(count == uint(journal["admitted_strokes"], 64, "stroke_census") and count >= target, "stroke_census")
    echo = journal["echo"]
    closed(echo, {"observed", "missing", "pending"}, "echo_coverage")
    require(uint(echo["observed"], U64, "echo_coverage") == len(samples)
            and uint(echo["missing"], U64, "echo_coverage") == 0
            and uint(echo["pending"], 1 << 31, "echo_coverage") == 0, "echo_coverage")
    require(uint(journal["acknowledged_through"], U64, "ack_coverage") >= samples[-1]["seq"], "ack_coverage")

    contact_samples = [s for s in samples if s["phase"] in ("Down", "Move", "Up")]
    flags_seen = {s["pen_flags"] for s in contact_samples}
    kind = block.removeprefix("paint-")
    span = hardware_shape(events, samples, census, kind)
    if kind == "pressure":
        require(flags_seen <= {0}, "block_flags")
        require(count >= PRESSURE_MIN_STROKES, "pressure_strokes")
        require(span >= PRESSURE_MIN_SPAN, "pressure_span")
        pressures = [s["raw_pressure"] for s in samples if s["phase"] in ("Down", "Move")]
        require(statistics.pstdev(pressures) >= PRESSURE_MIN_STDEV, "pressure_variance")
    elif kind == "hover":
        require(not contact_samples, "hover_contact")
        require(any(s["phase"] == "Hover" for s in samples), "hover_not_observed")
    elif kind == "barrel":
        require(flags_seen <= {0, PEN_FLAG_BARREL}, "block_flags")
        require(any(s["pen_flags"] & PEN_FLAG_BARREL for s in contact_samples), "barrel_not_observed")
    elif kind == "eraser":
        require(flags_seen <= {0, PEN_FLAG_ERASER, PEN_FLAG_ERASER | PEN_FLAG_BARREL}, "block_flags")
        require(any(s["pen_flags"] & PEN_FLAG_ERASER for s in contact_samples), "eraser_not_observed")
    else:  # eraser-tool: Paint's Eraser tool with an ordinary pen tip.
        require(flags_seen <= {0}, "block_flags")
    return {
        "block": block, "target_strokes": target, "strokes": count, "samples": len(samples), "events": len(events),
        "contact_samples": len(contact_samples), "hover_samples": sum(s["phase"] == "Hover" for s in samples),
        "barrel_samples": sum(bool(s["pen_flags"] & PEN_FLAG_BARREL) for s in samples),
        "eraser_samples": sum(bool(s["pen_flags"] & PEN_FLAG_ERASER) for s in samples),
        "raw_pressure_span": span, "device_model": journal["device_model"], "sdk": journal["sdk"],
        "window_deliveries": deliveries, "synthetic_dispatches": 0,
        "journal_sha256": hashlib.sha256(raw).hexdigest(), "journal_bytes": len(raw),
        "contact_strokes": contact_strokes,
    }


def validate_record(records, views, journal, raw):
    require(isinstance(records, list) and len(records) == 1, "record_shape")
    r = records[0]
    require(isinstance(r, dict) and set(r) == RECORD_KEYS
            and all(isinstance(v, str) and len(v) <= 2048 for v in r.values()), "record_shape")
    route = r["remote_physical_route"]
    require("software_generated=true" not in route, "software_route_refused")
    require(route == ROUTE, "record_route")
    binding = r["remote_physical_binding"].split(",")
    require(len(binding) == 6, "record_binding")
    require(binding[:5] == views[0]["remote_render_scope"].split(","), "record_scope")
    require(UUID.fullmatch(binding[5]) is not None, "record_binding")
    require(r["remote_physical_owner"] == views[0]["remote_render_owner"], "record_owner")
    samples = journal["samples"]
    require(r["remote_physical_block"] == journal["block"], "record_journal")
    require(r["remote_physical_strokes"] == f"{journal['admitted_strokes']},{journal['target_strokes']}", "record_journal")
    require(r["remote_physical_samples"] == str(len(samples)), "record_journal")
    require(r["remote_physical_sequences"] == f"1,{samples[-1]['seq']}", "record_journal")
    echo = journal["echo"]
    require(r["remote_physical_echo"] == f"{echo['observed']},{echo['missing']},{echo['pending']}", "record_journal")
    require(r["remote_physical_witness"] == f"{journal['window_deliveries']},{journal['synthetic_dispatches']}", "record_journal")
    digest, _, size = r["remote_physical_journal"].partition(",")
    require(SHA.fullmatch(digest) is not None and digest == hashlib.sha256(raw).hexdigest()
            and size == str(len(raw)), "record_journal")
    ack = r["remote_physical_ack"].split(",")
    require(len(ack) == 3, "record_ack")
    frame, ticket, applied = integer(ack[0]), integer(ack[1]), integer(ack[2])
    require(frame > integer(views[0]["remote_render_frame"]) and ticket != integer(views[0]["remote_render_ticket"]), "record_ack")
    require(applied == journal["acknowledged_through"] and applied >= samples[-1]["seq"], "record_ack")
    return {"binding_scope_matches_first_view": True, "decoder_owner_matches_first_view": True,
            "covering_frame": frame, "acknowledged_through": applied}


def receiver_phase(flags):
    """Lifecycle phase from actual pointer flags (tools/pen-inject/analyze.py)."""
    return _analyze.phase(flags)


def validate_receiver(counts, events, journal, summary):
    require(isinstance(counts, list) and len(counts) == 6, "receiver_counts")
    counts = [uint(v, 20000, "receiver_counts") for v in counts]
    require(counts[3:] == [0, 0, 0], "receiver_mouse")
    samples = journal["samples"]
    downs = sum(s["phase"] == "Down" for s in samples)
    ups = sum(s["phase"] == "Up" for s in samples)
    require(counts[0] == downs and counts[2] == ups, "receiver_counts")
    require(isinstance(events, list) and 1 <= len(events) <= 20000 and len(events) == sum(counts), "receiver_counts")
    rows, seen, pointer = [], set(), None
    tally = [0] * 3
    for e in events:
        require(isinstance(e, dict) and RECEIVER_KEYS <= set(e) <= RECEIVER_KEYS | {"received_qpc", "input_provenance", "pen_history"},
                "receiver_event_shape")
        index = uint(e["index"], 5, "receiver_event_shape")
        require(index < 3, "receiver_mouse")
        tally[index] += 1
        require(uint(e["count"], 20000, "receiver_event_shape") == tally[index], "receiver_event_shape")
        require(e["pointer_available"] is True, "receiver_event_shape")
        identity = uint(e["pointer_id"], 65535, "receiver_event_shape", 1)
        require(pointer is None or identity == pointer, "receiver_event_shape")
        pointer = identity
        if "input_provenance" in e:
            try:
                validate_provenance(e)
            except (ValueError, KeyError, TypeError):
                raise ValueError("receiver_provenance_shape") from None
        history = e.get("pen_history")
        require(isinstance(history, list) and 1 <= len(history) <= 256, "receiver_history")
        for h in history:
            closed(h, HISTORY_KEYS, "receiver_history")
            sint(h["x"], -(1 << 31), (1 << 31) - 1, "receiver_history")
            sint(h["y"], -(1 << 31), (1 << 31) - 1, "receiver_history")
            uint(h["pressure"], 1024, "receiver_history")
            sint(h["tilt_x"], -90, 90, "receiver_history")
            sint(h["tilt_y"], -90, 90, "receiver_history")
            uint(h["rotation"], 359, "receiver_history")
            uint(h["pen_flags"], (1 << 32) - 1, "receiver_history")
            uint(h["pen_mask"], (1 << 32) - 1, "receiver_history")
            uint(h["pointer_flags"], (1 << 32) - 1, "receiver_history")
            uint(h["frame_id"], (1 << 32) - 1, "receiver_history")
            uint(h["performance_count"], U64, "receiver_history")
            uint(h["history_count"], (1 << 32) - 1, "receiver_history")
            key = (identity, h["frame_id"], h["performance_count"], h["pointer_flags"])
            if key in seen:
                continue  # Overlapping coalesced history across messages.
            seen.add(key)
            rows.append(dict(h, ordinal=len(rows), phase=receiver_phase(h["pointer_flags"])))
    require(tally == counts[:3], "receiver_counts")

    buckets = collections.defaultdict(collections.deque)
    for row in rows:
        buckets[(row["x"], row["y"], row["phase"])].append(row)
    mapping = {"Down": "down", "Move": "move", "Up": "up", "Hover": "hover"}
    matched, pairs, order, used = [], [], [], set()
    missing_contact = missing_hover = 0
    for s in samples:
        phase = mapping.get(s["phase"])
        if phase is None:
            continue  # Leave carries no INRANGE frame to correlate.
        queue = buckets[(s["host_x"], s["host_y"], phase)]
        if not queue:
            if phase == "hover":
                missing_hover += 1
            else:
                missing_contact += 1
            continue
        row = queue.popleft()
        used.add(row["ordinal"])
        order.append(row["ordinal"])
        matched.append((s, row))
        if phase in ("down", "move"):
            pairs.append((s["raw_pressure"], row["pressure"]))
    require(missing_contact == 0, "receiver_missing_contact")
    require(missing_hover == 0, "receiver_missing_hover")
    require(order == sorted(order), "receiver_order")
    # Only lifecycle frames the phone never sent may stay unmatched: an injected
    # Leave (no INRANGE) and Windows' noncontact update repeating an UP position.
    last_up, previous, os_noncontact, duplicate_hover = None, None, 0, 0
    for row in rows:
        here = (row["x"], row["y"], row["pen_flags"])
        if row["phase"] == "up":
            last_up = here
        tolerated = False
        if row["ordinal"] in used or row["phase"] == "leave":
            tolerated = True
        elif row["phase"] == "hover" and row["pressure"] == 0 and last_up == here:
            # Windows' noncontact update repeating the UP position; a held barrel
            # or eraser flag stays on it.
            os_noncontact += 1
            tolerated = True
        elif (row["phase"] == "hover" and row["pressure"] == 0 and previous is not None
              and previous["phase"] == "hover" and (previous["x"], previous["y"], previous["pen_flags"]) == here):
            duplicate_hover += 1
            tolerated = True
        if not tolerated:
            raise ValueError("receiver_unmatched")
        previous = row
    for s, row in matched:
        require(row["pen_flags"] == s["pen_flags"], "receiver_flags")
        require(row["pressure"] == s["sent_pressure"], "receiver_pressure")
        if s["phase"] == "Hover":
            require(row["pointer_flags"] & INRANGE and not row["pointer_flags"] & INCONTACT, "receiver_hover")

    r = pearson(pairs)
    raw_values = [x for x, _ in pairs]
    span = (max(raw_values) - min(raw_values)) if raw_values else 0.0
    kind = summary["block"]
    if kind == "pressure":
        require(len(pairs) >= PRESSURE_MIN_PAIRS, "pressure_pairs")
        require(r is not None, "constant_pressure")
        # Owner variation is the journal-level span/variance; r here only shows
        # the receiver saw the pressure the phone sent (transport fidelity).
        require(span >= PRESSURE_MIN_SPAN, "pressure_span")
        require(r >= TRANSPORT_MIN_R, "transport_pressure_fidelity")
        stroke_rows = sum(1 for stroke in summary["contact_strokes"]
                          if sum(s["phase"] in ("Down", "Move") for s in stroke) >= 2)
        require(stroke_rows >= PRESSURE_MIN_STROKES, "pressure_strokes")
    elif kind == "hover":
        require(counts[0] == 0 and counts[2] == 0, "hover_contact")
        require(any(s["phase"] == "Hover" for s, _ in matched), "hover_not_observed")
    elif kind == "barrel":
        require(any(row["pen_flags"] & PEN_FLAG_BARREL and row["pointer_flags"] & INCONTACT for _, row in matched), "barrel_not_observed")
    elif kind == "eraser":
        require(any(row["pen_flags"] & PEN_FLAG_ERASER and row["pointer_flags"] & INCONTACT for _, row in matched), "eraser_not_observed")
    return {"receiver_messages": len(events), "receiver_frames": len(rows), "matched_frames": len(matched),
            "os_noncontact_after_up": os_noncontact, "os_duplicate_hover": duplicate_hover, "pressure_pairs": len(pairs),
            "transport_pressure_r": r, "pressure_pair_raw_span": span,
            "receiver_counts": counts}


def validate(mode, journal, raw, records, views, counts=None, receiver=None, run_id=None):
    require(mode in ("harness", "phone"), "arguments")
    summary = validate_journal(journal, raw)
    require(isinstance(run_id, str) and RUN.fullmatch(run_id) is not None and journal["run_id"] == run_id, "run_binding")
    require((summary["block"] in HARNESS_BLOCKS) == (mode == "harness"), "receiver_mode")
    validate_views(views)
    record = validate_record(records, views, journal, raw)
    result = {key: value for key, value in summary.items() if key != "contact_strokes"}
    result.update(record)
    if mode == "harness":
        result.update(validate_receiver(counts, receiver, journal, summary))
    else:
        result.update(receiver_observed=False)
    result.update(schema=1, mode=mode, run_id=run_id, software_generated_input=False, physical_owner_gesture=True,
                  physical_claim=PHYSICAL_CLAIM, text_correlation_only=True, latency_acceptance=False,
                  editor_effect=False, tilt_calibrated=False)
    return result


def evidence(journal, raw):
    """Bounded closed observation retained even on refusal; never acceptance."""
    out = {"schema": 1, "journal_bytes": len(raw) if raw is not None else None,
           "journal_sha256": hashlib.sha256(raw).hexdigest() if raw is not None else None,
           "acceptance": False}
    if not isinstance(journal, dict):
        return out
    block = journal.get("block")
    out["block"] = block if block in BLOCKS else None
    for key in ("complete", "synthetic_dispatches", "window_deliveries", "admitted_strokes", "target_strokes"):
        value = journal.get(key)
        out[key] = value if type(value) in (bool, int) and (type(value) is bool or 0 <= value <= I64) else None
    reason = journal.get("stop_reason")
    out["stop_reason"] = reason if reason in ("target_reached", "sample_limit", "event_limit", "device_limit", "stopped", "recording") else None
    for name in ("events", "samples"):
        rows = journal.get(name)
        rows = rows if isinstance(rows, list) else []
        out[name + "_count"] = len(rows)
        tally = collections.Counter(r.get("origin") for r in rows if isinstance(r, dict))
        out[name + "_origins"] = {origin: tally.get(origin, 0) for origin in ORIGINS}
    pauses = journal.get("owner_pauses")
    out["owner_pauses"] = [{"reason": p["reason"], "count": p["count"]} for p in (pauses if isinstance(pauses, list) else [])[:16]
                           if isinstance(p, dict) and isinstance(p.get("reason"), str) and re.fullmatch(r"[a-z_]{1,32}", p["reason"])
                           and type(p.get("count")) is int and 0 <= p["count"] <= I64]
    return out


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def nonfinite(_):
    raise ValueError("nonfinite_json")


def read_bytes(path, limit):
    p = Path(path)
    require(p.is_file() and not p.is_symlink() and 0 < p.stat().st_size <= limit, "receipt_file")
    with p.open("rb") as stream:
        raw = stream.read(limit + 1)
    require(0 < len(raw) <= limit, "receipt_file_bytes")
    return raw


def parse(raw):
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=unique, parse_constant=nonfinite)
    except UnicodeDecodeError:
        raise ValueError("json_shape") from None


def parse_lines(raw):
    out = []
    for line in raw.decode("utf-8").splitlines():
        if line:
            out.append(json.loads(line, object_pairs_hook=unique, parse_constant=nonfinite))
    return out


def label(error):
    value = error.args[0] if len(error.args) == 1 and isinstance(error.args[0], str) else None
    return value if value in REASONS else "unclassified"


def report(mode, journal, raw, records, views, counts=None, receiver=None, run_id=None):
    result = {"schema": 1, "operation": "remote-physical-gesture", "mode": mode if mode in ("harness", "phone") else None,
              "status": "rejected", "rejection_reason": "unclassified", "records_validated": False,
              "physical_owner_gesture": False, "software_generated_input": False, "latency_acceptance": False,
              "evidence": evidence(journal, raw)}
    try:
        validation = validate(mode, journal, raw, records, views, counts, receiver, run_id)
    except (ValueError, TypeError, KeyError, OverflowError, RecursionError) as error:
        result["rejection_reason"] = label(error)
    else:
        result.update(status="passed", rejection_reason="none", records_validated=True,
                      physical_owner_gesture=True, validation=validation)
    return result


def main(arguments):
    journal = raw = None
    mode = arguments[0] if arguments else None
    try:
        require((mode == "harness" and len(arguments) == 7) or (mode == "phone" and len(arguments) == 5), "arguments")
        run_id = arguments[1]
        require(RUN.fullmatch(run_id) is not None, "arguments")
        raw = read_bytes(arguments[2], JOURNAL_LIMIT)
        journal = parse(raw)
        records = parse(read_bytes(arguments[3], RECORD_LIMIT))
        views = parse(read_bytes(arguments[4], RECORD_LIMIT))
        counts = receiver = None
        if mode == "harness":
            counts = parse(read_bytes(arguments[5], 128))
            receiver = parse_lines(read_bytes(arguments[6], RECEIVER_LIMIT))
        result = report(mode, journal, raw, records, views, counts, receiver, run_id)
    except (ValueError, OSError, TypeError, KeyError, OverflowError, RecursionError, UnicodeDecodeError) as error:
        result = report(mode, journal, raw, None, None)
        result["rejection_reason"] = label(error)
    print(json.dumps(result, separators=(",", ":"), allow_nan=False))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
