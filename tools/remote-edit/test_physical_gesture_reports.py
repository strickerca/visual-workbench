import copy
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import controller_input_reports
import physical_gesture_reports as reports

HERE = Path(__file__).resolve().parent
RUN = "0123456789abcdef0123456789abcdef"
DOWN, MOVE, UP, HOVER, LEAVE = 0x12006, 0x22006, 0x42002, 0x22002, 0x20000
ACTIONS = {"Down": 0, "Up": 1, "Move": 2, "Hover": 7, "Leave": 10}


def ident(n):
    return f"01a10700-0000-7000-8000-{n:012x}"


def views():
    rows = []
    for i in range(3):
        rows.append({"remote_render_scope": f"{1 if i < 2 else 2},{ident(i + 1)},1,{ident(10)},1",
                     "remote_render_owner": ident(i + 20), "remote_render_ticket": str(5 + i),
                     "remote_render_frame": str(10 + i), "remote_render_pts_us": str(100000 + i * 1000),
                     "remote_render_codec": "c2.qti.hevc.decoder",
                     "remote_render_timing": f"{900000000 + i * 1000},{910000000 + i * 1000}",
                     "remote_render_capabilities": "hardware=true,low_latency_advertised=false,requested=true,configure_accepted=true"})
    return rows


def event(index, tool=2, buttons=0, delivered=True, origin="PhysicalStylus", device=7, phase="Move", history=0, source=0x5002):
    action = ACTIONS[phase]
    verification = "NotDelivered" if not delivered else ("Verified" if action in (0, 1) else "NotSigned")
    return {"event": index, "action": action, "device_id": device, "device_virtual": False, "declares_stylus": True,
            "external": False, "device_sources": 0x5002, "source": source, "tool_type": tool, "flags": 0, "button_state": buttons,
            "action_button": 0, "classification": 0, "history_size": history, "event_time_ns": 1_000_000 + index,
            "window_delivered": delivered, "verification": verification, "origin": origin}


class Builder:
    """Builds a consistent phone journal, status record and receiver journal."""

    def __init__(self, block="pressure", strokes=6, moves=39, hovers=3, flags=0, tool=2, buttons=0):
        self.block, self.target = block, strokes
        self.events, self.samples, self.receiver = [], [], []
        self.counts = [0, 0, 0, 0, 0, 0]
        x = 100
        for stroke in range(strokes):
            y = 300 + stroke
            # Kotlin derives pen flags per event, so hover samples carry them too.
            for _ in range(hovers):
                self.add("Hover", x, y, 0.0, flags, tool, buttons)
                x += 1
            self.add("Leave", x, y, 0.0, flags, tool, buttons)
            if block in reports.HOVER_BLOCKS:
                continue
            for step in range(moves + 1):
                pressure = 0.05 + 0.9 * step / moves
                self.add("Down" if step == 0 else "Move", x, y, pressure, flags, tool, buttons)
                x += 1
            self.add("Up", x - 1, y, 0.0, flags, tool, buttons)

    def add(self, phase, x, y, pressure, flags, tool, buttons):
        index = len(self.events)
        # Hardware batching: every Move event carries one historical frame; every
        # other Move row is that historical sample.
        historical = phase == "Move" and len(self.samples) % 2 == 1
        self.events.append(event(index, tool, buttons, phase=phase, history=1 if phase in ("Move", "Hover") else 0))
        sent = min(1024, int(pressure * 1024 + 0.5))
        self.samples.append({"seq": len(self.samples) + 1, "event": index, "phase": phase, "host_x": x, "host_y": y,
                             "sent_pressure": sent, "raw_pressure": pressure, "raw_tilt": 0.25, "raw_orientation": 1.5,
                             "raw_distance": 0.0, "tilt_x": 3, "tilt_y": -4, "pen_mask": 15, "pen_flags": flags,
                             "sampled_ns": 2_000_000 + len(self.samples), "historical": historical, "origin": "PhysicalStylus"})
        pointer = {"Down": DOWN, "Move": MOVE, "Up": UP, "Hover": HOVER, "Leave": LEAVE}[phase]
        message_index = {"Down": 0, "Up": 2}.get(phase, 1)
        self.counts[message_index] += 1
        frame = len(self.receiver) + 1
        history = [{"x": x, "y": y, "pressure": 0 if phase == "Leave" else sent, "tilt_x": 3, "tilt_y": -4, "rotation": 0,
                    "pen_flags": flags, "pen_mask": 7, "pointer_flags": pointer,
                    "frame_id": frame, "performance_count": 5000 + frame, "history_count": 1}]
        self.receiver.append({"message": [0x246, 0x245, 0x247][message_index], "index": message_index,
                              "count": self.counts[message_index], "pointer_available": True, "pressure": history[0]["pressure"],
                              "pen_flags": history[0]["pen_flags"], "performance_count": 5000 + frame,
                              "pointer_flags": pointer, "pointer_id": 1, "pen_history": history})

    def journal(self):
        strokes, _ = reports.strokes(self.samples, self.block in reports.HOVER_BLOCKS)
        return {"schema": 1, "kind": "remote-physical-gesture-journal", "run_id": "0123456789abcdef0123456789abcdef",
                "block": self.block, "target_strokes": self.target, "hover_only": self.block in reports.HOVER_BLOCKS,
                "device_model": "SM-S918U", "sdk": 36, "clock": "android_uptime_ns", "timestamp_resolution_ns": 1,
                "complete": True, "stop_reason": "target_reached", "synthetic_dispatches": 0,
                "window_deliveries": len(self.events) + 5, "admitted_strokes": strokes,
                "acknowledged_through": len(self.samples),
                "echo": {"observed": len(self.samples), "missing": 0, "pending": 0},
                "input_devices": [{"device_id": 7, "sources": 0x5002, "declares_stylus": True, "virtual": False,
                                   "external": False, "ranges": [{"axis": 2, "source": 0x5002, "min": 0.0, "max": 1.0,
                                                                  "flat": 0.0, "fuzz": 0.0, "resolution": 0.0}]}],
                "events": copy.deepcopy(self.events), "samples": copy.deepcopy(self.samples), "owner_pauses": []}


def encode(journal):
    return json.dumps(journal, separators=(",", ":")).encode()


def record(journal, raw, render):
    samples = journal["samples"]
    echo = journal["echo"]
    return [{"remote_physical_binding": render[0]["remote_render_scope"] + "," + ident(40),
             "remote_physical_block": journal["block"],
             "remote_physical_strokes": f"{journal['admitted_strokes']},{journal['target_strokes']}",
             "remote_physical_samples": str(len(samples)), "remote_physical_sequences": f"1,{samples[-1]['seq']}",
             "remote_physical_owner": render[0]["remote_render_owner"],
             "remote_physical_ack": f"12,9,{journal['acknowledged_through']}",
             "remote_physical_echo": f"{echo['observed']},{echo['missing']},{echo['pending']}",
             "remote_physical_witness": f"{journal['window_deliveries']},{journal['synthetic_dispatches']}",
             "remote_physical_journal": f"{hashlib.sha256(raw).hexdigest()},{len(raw)}",
             "remote_physical_route": reports.ROUTE}]


class PhysicalGestureReportsTest(unittest.TestCase):
    def run_harness(self, builder, journal_change=None, receiver_change=None, record_change=None):
        journal = builder.journal()
        if journal_change:
            journal_change(journal)
        raw = encode(journal)
        render = views()
        records = record(journal, raw, render)
        if record_change:
            record_change(records[0])
        counts, receiver = list(builder.counts), copy.deepcopy(builder.receiver)
        if receiver_change:
            receiver_change(counts, receiver)
        return reports.report("harness", journal, raw, records, render, counts, receiver, RUN)

    def refused(self, reason, *args, **kwargs):
        result = self.run_harness(*args, **kwargs)
        self.assertEqual("rejected", result["status"], result)
        self.assertEqual(reason, result["rejection_reason"])
        self.assertFalse(result["physical_owner_gesture"])
        self.assertFalse(result["evidence"]["acceptance"])

    def test_exact_physical_pressure_route_passes_with_correlation(self):
        result = self.run_harness(Builder())
        self.assertEqual("passed", result["status"], result)
        v = result["validation"]
        self.assertGreaterEqual(v["pressure_pairs"], 200)
        self.assertGreaterEqual(v["transport_pressure_r"], 0.95)
        self.assertEqual(reports.PHYSICAL_CLAIM, v["physical_claim"])
        self.assertTrue(v["physical_claim"].endswith("not_proof_against_crafted_privileged_injection"))
        self.assertGreaterEqual(v["strokes"], 5)
        self.assertTrue(v["physical_owner_gesture"])
        self.assertFalse(v["software_generated_input"])
        self.assertFalse(v["latency_acceptance"])
        self.assertFalse(v["tilt_calibrated"])
        self.assertNotIn("contact_strokes", v)

    def test_synthetic_direct_dispatch_claiming_physical_is_refused(self):
        def direct(j):
            j["events"][5].update(window_delivered=False, verification="NotDelivered", origin="Direct")
            for s in j["samples"]:
                if s["event"] == 5:
                    s["origin"] = "Direct"
        self.refused("non_physical_origin", Builder(), direct)
        # The same direct dispatch while still labelled physical is a forged origin.
        self.refused("origin_mismatch", Builder(), lambda j: j["events"][5].update(window_delivered=False, verification="NotDelivered"))
        self.refused("synthetic_dispatch", Builder(), lambda j: j.update(synthetic_dispatches=1))
        self.refused("software_route_refused", Builder(),
                     record_change=lambda r: r.update(remote_physical_route=controller_input_reports.ROUTE))
        self.refused("record_route", Builder(),
                     record_change=lambda r: r.update(remote_physical_route=reports.ROUTE.replace("physical_owner_gesture=true", "physical_owner_gesture=false")))

    def test_injected_and_unknown_devices_are_refused(self):
        self.refused("origin_mismatch", Builder(), lambda j: j["events"][0].update(device_id=-1))
        self.refused("origin_mismatch", Builder(), lambda j: j["events"][0].update(flags=0x800))
        self.refused("origin_mismatch", Builder(), lambda j: j["events"][0].update(tool_type=1))
        self.refused("device_census", Builder(), lambda j: j["input_devices"][0].update(virtual=True))

    def test_synthetic_validator_still_refuses_any_pen_flags(self):
        inputs = [{"remote_input_binding": views()[0]["remote_render_scope"] + "," + ident(40),
                   "remote_input_sequences": "1,3", "remote_input_owner": views()[0]["remote_render_owner"],
                   "remote_input_frame": "12", "remote_input_ticket": "9", "remote_input_pts_us": "120000",
                   "remote_input_timing": "1000000000,1100000000,1110000000,1110000000,1270000000",
                   "remote_input_echo": "3,0,0", "remote_input_route": controller_input_reports.ROUTE}]
        events = [{"message": [0x246, 0x245, 0x247][i], "index": i, "count": 1, "pointer_available": True,
                   "pressure": 512 if i < 2 else 0, "pen_flags": 1, "performance_count": 1000 + i,
                   "pointer_flags": [0x12017, 0x22016, 0x42002][i], "pointer_id": 1} for i in range(3)]
        with self.assertRaises(ValueError):
            controller_input_reports.validate(inputs, views(), [1, 1, 1, 0, 0, 0], events)

    def test_constant_or_narrow_pressure_is_refused(self):
        def constant(j):
            for s in j["samples"]:
                if s["phase"] in ("Down", "Move"):
                    s.update(raw_pressure=0.5, sent_pressure=512)
        self.refused("constant_pressure", Builder(), constant)

        def narrow(j):
            for s in j["samples"]:
                if s["phase"] in ("Down", "Move"):
                    s["raw_pressure"] = 0.4 + s["raw_pressure"] * 0.2
                    s["sent_pressure"] = int(s["raw_pressure"] * 1024 + 0.5)
        self.refused("pressure_span", Builder(), narrow)

    def test_too_few_pairs_or_strokes_are_refused(self):
        self.refused("pressure_pairs", Builder(strokes=5, moves=20))
        self.refused("pressure_strokes", Builder(strokes=4, moves=80))

    def test_journal_completeness_pauses_and_sequences(self):
        self.refused("journal_incomplete", Builder(), lambda j: j.update(complete=False, stop_reason="sample_limit"))
        self.refused("owner_pause", Builder(), lambda j: j.update(owner_pauses=[{"reason": "tool_type", "count": 1}]))
        self.refused("sequence_census", Builder(), lambda j: j["samples"][10].update(seq=99))
        self.refused("stroke_census", Builder(), lambda j: j.update(admitted_strokes=7))
        self.refused("stroke_census", Builder(), lambda j: j.update(target_strokes=7))
        self.refused("phase_transition", Builder(), lambda j: j["samples"][4].update(phase="Up"))
        self.refused("sample_pressure", Builder(), lambda j: j["samples"][6].update(sent_pressure=1000))
        self.refused("journal_shape", Builder(), lambda j: j.update(trusted=True))
        self.refused("sample_shape", Builder(), lambda j: j["samples"][6].update(historical=True, phase="Hover"))

    def test_echo_and_ack_coverage_are_required(self):
        self.refused("echo_coverage", Builder(), lambda j: j["echo"].update(missing=1))
        self.refused("echo_coverage", Builder(), lambda j: j["echo"].update(observed=1))
        self.refused("ack_coverage", Builder(), lambda j: j.update(acknowledged_through=3))
        self.refused("record_ack", Builder(), record_change=lambda r: r.update(remote_physical_ack="10,9,999"))

    def test_record_binds_exact_journal_bytes_scope_and_owner(self):
        self.refused("record_journal", Builder(),
                     record_change=lambda r: r.update(remote_physical_journal="0" * 64 + ",10"))
        self.refused("record_scope", Builder(),
                     record_change=lambda r: r.update(remote_physical_binding=views()[2]["remote_render_scope"] + "," + ident(40)))
        self.refused("record_owner", Builder(),
                     record_change=lambda r: r.update(remote_physical_owner=views()[1]["remote_render_owner"]))
        self.refused("record_journal", Builder(), record_change=lambda r: r.update(remote_physical_samples="1"))

    def test_receiver_correlation_is_exact(self):
        def drop_contact(counts, receiver):
            receiver.pop(8)
            counts[1] -= 1
            for i, e in enumerate(r for r in receiver if r["index"] == 1):
                e["count"] = i + 1
        self.refused("receiver_missing_contact", Builder(), receiver_change=drop_contact)
        self.refused("receiver_flags", Builder(), receiver_change=lambda c, r: r[8]["pen_history"][0].update(pen_flags=1))
        self.refused("receiver_pressure", Builder(), receiver_change=lambda c, r: r[8]["pen_history"][0].update(pressure=3))
        self.refused("receiver_mouse", Builder(), receiver_change=lambda c, r: c.__setitem__(4, 1))
        self.refused("receiver_history", Builder(), receiver_change=lambda c, r: r[8].update(pen_history=None))

        def extra_contact(counts, receiver):
            extra = copy.deepcopy(receiver[8])
            extra["pen_history"][0].update(x=9999, frame_id=99999, performance_count=99999)
            counts[1] += 1
            extra["count"] = counts[1]
            receiver.append(extra)
        self.refused("receiver_unmatched", Builder(), receiver_change=extra_contact)

    def test_coalesced_history_expands_and_noncontact_after_up_is_tolerated(self):
        def coalesce(counts, receiver):
            # Fold receiver messages 6 and 7 into message 8's history (newest last).
            merged = receiver[6]["pen_history"] + receiver[7]["pen_history"] + receiver[8]["pen_history"]
            receiver[8]["pen_history"] = merged
            del receiver[6:8]
            counts[1] -= 2
            for i, e in enumerate(r for r in receiver if r["index"] == 1):
                e["count"] = i + 1
            up = next(i for i, e in enumerate(receiver) if e["index"] == 2)
            after = copy.deepcopy(receiver[up])
            after.update(index=1, message=0x245, pointer_flags=HOVER)
            after["pen_history"][0].update(pointer_flags=HOVER, frame_id=88888, performance_count=88888)
            receiver.insert(up + 1, after)
            counts[1] += 1
            for i, e in enumerate(r for r in receiver if r["index"] == 1):
                e["count"] = i + 1
        result = self.run_harness(Builder(), receiver_change=coalesce)
        self.assertEqual("passed", result["status"], result)
        self.assertEqual(1, result["validation"]["os_noncontact_after_up"])

    def test_hover_block_requires_inrange_without_contact(self):
        result = self.run_harness(Builder("hover", strokes=3, hovers=6))
        self.assertEqual("passed", result["status"], result)
        self.assertEqual(0, result["validation"]["contact_samples"])

        def touched(j):
            j["samples"][1].update(phase="Down")
        self.refused("phase_transition", Builder("hover", strokes=3, hovers=6), touched)
        # A receiver contact frame can never stand in for a phone hover sample.
        self.refused("receiver_missing_hover", Builder("hover", strokes=3, hovers=6),
                     receiver_change=lambda c, r: r[0]["pen_history"][0].update(pointer_flags=MOVE))

    def test_barrel_and_eraser_flags_admitted_only_when_observed(self):
        result = self.run_harness(Builder("barrel", strokes=2, moves=10, flags=1, buttons=0x20))
        self.assertEqual("passed", result["status"], result)
        self.assertGreater(result["validation"]["barrel_samples"], 0)
        self.refused("barrel_not_observed", Builder("barrel", strokes=2, moves=10))
        self.refused("sample_flags", Builder("barrel", strokes=2, moves=10, flags=1, buttons=0))
        result = self.run_harness(Builder("eraser", strokes=2, moves=10, flags=4, tool=4))
        self.assertEqual("passed", result["status"], result)
        self.refused("eraser_not_observed", Builder("eraser", strokes=2, moves=10))
        self.refused("block_flags", Builder("pressure", flags=1, buttons=0x20))

    def test_paint_blocks_use_phone_mode_only(self):
        builder = Builder("paint-pressure")
        journal = builder.journal()
        raw = encode(journal)
        render = views()
        result = reports.report("phone", journal, raw, record(journal, raw, render), render, run_id=RUN)
        self.assertEqual("passed", result["status"], result)
        self.assertFalse(result["validation"]["receiver_observed"])
        self.assertEqual("run_binding", reports.report("phone", journal, raw, record(journal, raw, render), render, run_id="f" * 32)["rejection_reason"])
        harness = Builder()
        journal = harness.journal()
        raw = encode(journal)
        result = reports.report("phone", journal, raw, record(journal, raw, render), render, run_id=RUN)
        self.assertEqual("receiver_mode", result["rejection_reason"])
        tool = Builder("paint-eraser-tool", strokes=2, moves=10)
        journal = tool.journal()
        raw = encode(journal)
        self.assertEqual("passed", reports.report("phone", journal, raw, record(journal, raw, render), render, run_id=RUN)["status"])

    def test_cli_reads_bounded_files_and_reports_closed_json(self):
        builder = Builder()
        journal = builder.journal()
        raw = encode(journal)
        render = views()
        with tempfile.TemporaryDirectory() as temp:
            temp = Path(temp)
            paths = {name: temp / name for name in ("journal.json", "records.json", "views.json", "counts.json", "received.jsonl")}
            paths["journal.json"].write_bytes(raw)
            paths["records.json"].write_text(json.dumps(record(journal, raw, render)))
            paths["views.json"].write_text(json.dumps(render))
            paths["counts.json"].write_text(json.dumps(builder.counts))
            paths["received.jsonl"].write_text("".join(json.dumps(e) + "\n" for e in builder.receiver))
            argv = [sys.executable, str(HERE / "physical_gesture_reports.py"), "harness", RUN] + [str(p) for p in paths.values()]
            done = subprocess.run(argv, capture_output=True, text=True, timeout=60, cwd=str(HERE))
            self.assertEqual(0, done.returncode, done.stdout + done.stderr)
            self.assertEqual("passed", json.loads(done.stdout)["status"])
            paths["journal.json"].write_bytes(raw.replace(b'"schema":1', b'"schema":1,"schema":1', 1))
            done = subprocess.run(argv, capture_output=True, text=True, timeout=60, cwd=str(HERE))
            self.assertEqual(1, done.returncode)
            self.assertEqual("duplicate_json_key", json.loads(done.stdout)["rejection_reason"])
            done = subprocess.run(argv[:5], capture_output=True, text=True, timeout=60, cwd=str(HERE))
            self.assertEqual("arguments", json.loads(done.stdout)["rejection_reason"])

    def test_kotlin_classifier_mirror_matches_documented_rules(self):
        base = event(0)
        down = event(0, phase="Down")
        self.assertEqual("PhysicalStylus", reports.classify(base, 0))
        self.assertEqual("PhysicalStylus", reports.classify(down, 0))
        self.assertEqual("Direct", reports.classify(dict(base, window_delivered=False), 0))
        self.assertEqual("Injected", reports.classify(dict(base, device_virtual=True), 0))
        self.assertEqual("Injected", reports.classify(dict(base, flags=0x8), 0))
        self.assertEqual("Injected", reports.classify(dict(down, verification="Refused"), 0))
        self.assertEqual("Unknown", reports.classify(base, 1))
        self.assertEqual("Unknown", reports.classify(dict(base, device_virtual=None), 0))
        self.assertEqual("Unknown", reports.classify(dict(base, source=0x1002), 0))
        self.assertEqual("Unknown", reports.classify(dict(base, source=0x4002), 0))
        self.assertEqual("Unknown", reports.classify(dict(base, device_sources=0x4002), 0))
        self.assertEqual("Unknown", reports.classify(dict(base, device_sources=None), 0))
        self.assertEqual("Unknown", reports.classify(dict(down, verification="Unavailable"), 0))
        self.assertEqual("Unknown", reports.classify(dict(down, verification="NotSigned"), 0))

    def test_injection_shaped_journals_are_refused_for_every_block(self):
        for block in reports.BLOCKS:
            hover_only = block in reports.HOVER_BLOCKS
            flags = 1 if block.endswith("barrel") else 4 if block.endswith("eraser") else 0
            buttons = 0x20 if flags == 1 else 0
            tool = 4 if flags == 4 else 2
            mode = "harness" if block in reports.HARNESS_BLOCKS else "phone"
            # `input stylus swipe`: real stylus-capable device id, signed DOWN/UP,
            # SOURCE_STYLUS only, constant pressure 1.0, no axes, no batching.
            for forged_source, label in ((0x4002, "PhysicalStylus"), (0x4002, "Unknown"), (0x5002, "PhysicalStylus")):
                builder = Builder(block, strokes=6, moves=39, flags=flags, tool=tool, buttons=buttons)
                journal = builder.journal()
                for e in journal["events"]:
                    e.update(source=forged_source, history_size=0, origin=label)
                for s in journal["samples"]:
                    s.update(origin=label, historical=False, raw_tilt=None, raw_orientation=None)
                    if s["phase"] in ("Down", "Move"):
                        s.update(raw_pressure=1.0, sent_pressure=1024)
                raw = encode(journal)
                render = views()
                result = reports.report(mode, journal, raw, record(journal, raw, render), render,
                                        list(builder.counts), copy.deepcopy(builder.receiver), RUN)
                self.assertEqual("rejected", result["status"], (block, forged_source, label))
                self.assertFalse(result["physical_owner_gesture"])
                if forged_source == 0x4002:
                    self.assertEqual("non_physical_origin" if label == "Unknown" else "origin_mismatch", result["rejection_reason"], block)
                else:
                    self.assertIn(result["rejection_reason"], {"hardware_history", "constant_pressure"}, block)

    def test_declared_axes_must_be_present_and_varying(self):
        declare = lambda j: j["input_devices"][0]["ranges"].extend(
            {"axis": axis, "source": 0x5002, "min": -1.0, "max": 1.0, "flat": 0.0, "fuzz": 0.0, "resolution": 0.0} for axis in (8, 24, 25))
        # The fixture's constant tilt/orientation cannot come from a hardware pen.
        self.refused("hardware_axes", Builder(), declare)

        def varying(j):
            declare(j)
            for i, s in enumerate(j["samples"]):
                s.update(raw_tilt=0.2 + (i % 7) * 0.01, raw_orientation=1.0 + (i % 5) * 0.02, raw_distance=0.0 if s["phase"] != "Hover" else 0.1 + (i % 3) * 0.05)
        self.assertEqual("passed", self.run_harness(Builder(), varying)["status"])

        def missing(j):
            varying(j)
            j["samples"][5]["raw_tilt"] = None
        self.refused("hardware_axes", Builder(), missing)

    def test_batched_history_and_contact_pressure_variation_are_required(self):
        def unbatched(j):
            for e in j["events"]:
                e["history_size"] = 0
            for s in j["samples"]:
                s["historical"] = False
        self.refused("hardware_history", Builder(), unbatched)

        def no_historical_rows(j):
            for s in j["samples"]:
                s["historical"] = False
        self.refused("hardware_history", Builder(), no_historical_rows)

        def constant(j):
            for s in j["samples"]:
                if s["phase"] in ("Down", "Move"):
                    s.update(raw_pressure=0.6, sent_pressure=614)
        for block, flags, buttons, tool in (("barrel", 1, 0x20, 2), ("eraser", 4, 0, 4)):
            self.refused("constant_pressure", Builder(block, strokes=2, moves=10, flags=flags, buttons=buttons, tool=tool), constant)

    def test_held_flag_noncontact_and_duplicate_hover_rows_are_tolerated(self):
        def after_up_with_flag(counts, receiver):
            up = next(i for i, e in enumerate(receiver) if e["index"] == 2)
            repeat = copy.deepcopy(receiver[up])
            repeat.update(index=1, message=0x245, pointer_flags=HOVER, pressure=0)
            repeat["pen_history"][0].update(pointer_flags=HOVER, pressure=0, frame_id=77777, performance_count=77777)
            receiver.insert(up + 1, repeat)
            first_hover = next(i for i, e in enumerate(receiver) if e["pointer_flags"] == HOVER and i != up + 1)
            duplicate = copy.deepcopy(receiver[first_hover])
            duplicate["pen_history"][0].update(frame_id=66666, performance_count=66666)
            receiver.insert(first_hover + 1, duplicate)
            counts[1] += 2
            for i, e in enumerate(r for r in receiver if r["index"] == 1):
                e["count"] = i + 1
        result = self.run_harness(Builder("barrel", strokes=2, moves=10, flags=1, buttons=0x20), receiver_change=after_up_with_flag)
        self.assertEqual("passed", result["status"], result)
        self.assertEqual((1, 1), (result["validation"]["os_noncontact_after_up"], result["validation"]["os_duplicate_hover"]))

        def after_up_wrong_flag(counts, receiver):
            after_up_with_flag(counts, receiver)
            up = next(i for i, e in enumerate(receiver) if e["index"] == 2)
            receiver[up + 1]["pen_history"][0]["pen_flags"] = 4
        self.refused("receiver_unmatched", Builder("barrel", strokes=2, moves=10, flags=1, buttons=0x20), receiver_change=after_up_wrong_flag)

    def test_receiver_rows_with_provenance_and_clock_metadata_are_admitted(self):
        def annotate(counts, receiver):
            for i, e in enumerate(receiver):
                e["received_qpc"] = {"schema": 1, "counter": 10_000 + i, "frequency": 10_000_000}
                e["input_provenance"] = {"schema": 1, "source_available": True, "device_type": 2, "origin_id": 2,
                                         "message_extra_info": "0000000000000000"}
        self.assertEqual("passed", self.run_harness(Builder(), receiver_change=annotate)["status"])
        self.refused("receiver_provenance_shape", Builder(),
                     receiver_change=lambda c, r: r[3].update(input_provenance={"schema": 1, "source_available": True}))


if __name__ == "__main__":
    unittest.main()
