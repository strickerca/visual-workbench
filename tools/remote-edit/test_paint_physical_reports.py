"""Offline checks of the owner-physical Paint compatibility evaluator."""
import copy
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import paint_physical_reports as reports
import physical_gesture_reports as gesture
import struct
import zlib
from test_physical_gesture_reports import event, record, views

HERE = Path(__file__).resolve().parent
KOTLIN_CASES = HERE.parents[1] / "apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/RemotePhysicalCompatibilityCases.kt"
CANVAS = {"x": 10, "y": 20, "width": 200, "height": 220}
BLACK, RED = (0, 0, 0), (200, 30, 30)
PRESSURE_ROWS = (20, 50, 80, 110, 140)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)


def png(pixels, width, height):
    """Unfiltered RGBA8 PNG with an explicit sRGB chunk (fast test encoder)."""
    stride = width * 4
    scan = b"".join(b"\x00" + pixels[y * stride:(y + 1) * stride] for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"sRGB", b"\x00") + chunk(b"IDAT", zlib.compress(scan, 1)) + chunk(b"IEND", b""))


def ident(n):
    return f"019f7c21-5678-7000-8000-{n:012x}"


def journal_for(block, strokes, flags=0, buttons=0, hover_only=False):
    """A valid phone journal for strokes given in canvas coordinates."""
    events, samples = [], []

    def add(phase, x, y, pressure):
        index = len(events)
        events.append(event(index, buttons=buttons))
        samples.append({"seq": len(samples) + 1, "event": index, "phase": phase, "host_x": CANVAS["x"] + x,
                        "host_y": CANVAS["y"] + y, "sent_pressure": min(1024, int(pressure * 1024 + 0.5)),
                        "raw_pressure": pressure, "raw_tilt": None, "raw_orientation": None, "raw_distance": 0.0,
                        "tilt_x": 0, "tilt_y": 0, "pen_mask": 1, "pen_flags": flags, "sampled_ns": 1000 + len(samples),
                        "historical": False, "origin": "PhysicalStylus"})
    for stroke in strokes:
        x0, y0, _ = stroke[0]
        add("Hover", x0, y0, 0.0)
        add("Leave", x0, y0, 0.0)
        if hover_only:
            for x, y, _ in stroke:
                add("Hover", x, y, 0.0)
            add("Leave", stroke[-1][0], stroke[-1][1], 0.0)
            continue
        for position, (x, y, pressure) in enumerate(stroke):
            add("Down" if position == 0 else "Move", x, y, pressure)
        add("Up", stroke[-1][0], stroke[-1][1], 0.0)
    count, _ = gesture.strokes(samples, hover_only)
    return {"schema": 1, "kind": "remote-physical-gesture-journal", "run_id": "0123456789abcdef0123456789abcdef",
            "block": block, "target_strokes": len(strokes), "hover_only": hover_only, "device_model": "SM-S918U", "sdk": 36,
            "clock": "android_uptime_ns", "timestamp_resolution_ns": 1, "complete": True, "stop_reason": "target_reached",
            "synthetic_dispatches": 0, "window_deliveries": len(events) + 3, "admitted_strokes": count,
            "acknowledged_through": len(samples), "echo": {"observed": len(samples), "missing": 0, "pending": 0},
            "input_devices": [{"device_id": 7, "sources": 0x5002, "declares_stylus": True, "virtual": False, "external": False, "ranges": []}],
            "events": events, "samples": samples, "owner_pauses": []}


def stroke(y, x0=10, x1=190, low=0.05, high=0.95):
    points = list(range(x0, x1 + 1, 2))
    return [(x, y, low + (high - low) * i / (len(points) - 1)) for i, x in enumerate(points)]


class PaintPhysicalReportsTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="paint-physical-evaluator-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.files = 0

    def pixels(self):
        return bytearray(b"\xff" * (CANVAS["width"] * CANVAS["height"] * 4))

    @staticmethod
    def paint(pixels, x, y, colour):
        if 0 <= x < CANVAS["width"] and 0 <= y < CANVAS["height"]:
            at = (y * CANVAS["width"] + x) * 4
            pixels[at:at + 3] = bytes(colour)

    def ink(self, pixels, strokes, colour, pressure_width=True, half=2):
        for s in strokes:
            for x, y, pressure in s:
                h = 1 + round(4 * pressure) if pressure_width else half
                for dy in range(-h, h + 1):
                    for dx in (0, 1):
                        self.paint(pixels, x + dx, y + dy, colour)

    def image(self, pixels):
        pixels = bytes(pixels)
        path = self.root / f"canvas-{self.files}.png"
        self.files += 1
        raw = png(pixels, CANVAS["width"], CANVAS["height"])
        path.write_bytes(raw)
        channels = [pixels[i::4] for i in range(4)]
        return str(path), {"png_sha256": hashlib.sha256(raw).hexdigest(), "png_blake3": "a" * 64, "png_bytes": len(raw),
                           "width": CANVAS["width"], "height": CANVAS["height"], "canvas_pixels": CANVAS["width"] * CANVAS["height"],
                           "canvas_offset_x": 0, "canvas_offset_y": 0, "rgba_min": [min(c) for c in channels],
                           "rgba_max": [max(c) for c in channels], "rgba_sum": [sum(c) for c in channels],
                           "alpha_not_opaque_pixels": 0, "decoded_rgba_sha256": hashlib.sha256(pixels).hexdigest(),
                           "explicit_srgb": True, "numeric_semantics_proved": False}

    def frame(self, metrics, start, sample):
        client = {"x": 0, "y": 0, "width": 2000, "height": 300}
        frame_rect = {"x": -5, "y": -20, "width": 2010, "height": 330}
        crop = {"x": CANVAS["x"] + 5, "y": CANVAS["y"] + 20, "width": CANVAS["width"], "height": CANVAS["height"]}
        native = {"schema": 1, "sampling_scope": "verified_editor_canvas", "operation": "owned-blank-editor-wgc-metrics", "status": "Measured",
                  "helper_sha256": "f" * 64, "helper_pid": 200 + sample, "helper_exit_code": 0, "suspended_before_job_assignment": True,
                  "exact_image_verified_before_resume": True, "process_and_job_retired": True, "io_worker_started": True,
                  "io_join_completed": True, "error": None, "retirement_error": None, "io_error": None, "capture_refusal": None,
                  "capture_color_depth": None, "target": copy.deepcopy(self.target), "image": copy.deepcopy(self.image_identity),
                  "source_proof": {"policy": "installed_paint", "held_process": True, "held_final_image": True, "held_ancestor_namespace": False,
                                   "installed_package_namespace": True, "namespace_lease_equivalence_claim": False,
                                   "input_authority": False, "profile_authority": False},
                  "installed_package_evidence": {"full_name": self.image_identity["packageFullName"], "version": self.image_identity["packageVersion"],
                                                 "status_ok": True, "status_flags": 0, "signature_kind": 3, "development": False,
                                                 "framework_or_resource_or_bundle_or_optional": False},
                  "capture_session_id": ident(20), "sample_id": ident(100 + sample), "before": self.witness(start), "after": self.witness(start + 6),
                  "frame": None, "metrics": None, "diagnostic_sha256": None, "input_sent": False, "profile_authority": False,
                  "numeric_semantics_proved": False, "image_file_presence": True, "root_image_disposal_required": True}
        native["canvas_frame"] = {"source_identity": {"capture_session_id": ident(20), "frame_id": 1, "geometry_revision": 1, "source_kind": "window",
                                                   "client_rect": client, "dpi_scale": 1.0, "monotonic_timestamp_ns": (start + 3) * 100,
                                                   "captured_at_ms": 0, "platform": "windows", "window_handle": 99, "monitor_id": ""},
                                  "target": {"window": 99, "process_id": 100, "process_created": 102, "client": client, "frame": frame_rect,
                                             "dpi": 96, "observed_ns": (start + 2) * 100},
                                  "canvas_rect_host": dict(CANVAS), "crop_in_frame": crop, "source_asset_id": "a" * 64,
                                  "png_bytes": metrics["png_bytes"], "width": CANVAS["width"], "height": CANVAS["height"], "bit_depth": 8,
                                  "border_visible": True, "lossless": True, "filename": "capture-canvas.png"}
        native["canvas_metrics"] = {"sampling_scope": "verified_editor_canvas", "image": metrics, "source_client_rect": client,
                                    "canvas_rect_host": dict(CANVAS), "crop_in_frame": crop}
        return native

    def witness(self, start, tool="Using Brush tool on Canvas", settings="d" * 64):
        return {"settingsDigest": settings, "canvasRuntimeIdHash": "e" * 64, "canvasRectHost": dict(CANVAS),
                "canvasToolName": tool, "observedStartQpc100ns": start, "observedEndQpc100ns": start + 1}

    def block(self, index, name, before, after, journal):
        base = 10 + 200 * index
        raw = json.dumps(journal, separators=(",", ":")).encode()
        journal_path = self.root / f"physical-journal-{index}.json"
        journal_path.write_bytes(raw)
        render = views()
        run = f"{index + 1:032x}"
        native = {"schema": 1, "kind": "normal-native-remote-usb-integration", "status": "passed", "model": "SM-S918U", "owner_user": 0,
                  "controller_input_scenario": "owner-physical", "physical_owner_block": name, "run_id": run, "retained_input_nonce": run,
                  "owned_editor_contract_sha256": "b" * 64, "physical_journal_sha256": hashlib.sha256(raw).hexdigest(),
                  "physical_journal_retained": f"physical-journal-{run}.json", "physical_records": record(journal, raw, render),
                  "phone_render_records": render, "physical_validation": {"journal_sha256": hashlib.sha256(raw).hexdigest(),
                                                                         "physical_owner_gesture": True, "block": name}}
        for key in ("controller_input_requested input_granted input_attempted input_dispatch_completed physical_owner_gesture "
                    "physical_records_validated phone_render_records_validated input_retirement_confirmed host_actual_retirement "
                    "phone_actual_retirement process_cleanup owned_packages_cleanup private_work_disposed "
                    "external_editor_destination_retained profiles_preserved").split():
            native[key] = True
        for key in ("software_generated_input editor_effect latency_acceptance controller_input_records_validated "
                    "editor_phone_records_validated held_lifecycle_records_validated").split():
            native[key] = False
        phone = {"schema": 1, "status": "phone-physical-correlated", "block": name, "native_receipt_path": "private-native.json",
                 "native_receipt_sha256": "c" * 64, "contract_sha256": "b" * 64, "journal_path": str(journal_path),
                 "journal_sha256": hashlib.sha256(raw).hexdigest(), "actual_remote_retirement": True, "retained_parent_returned": True,
                 "receiver_observed": False, "editor_effect": False, "local_history_may_begin": True, "physical_owner_gesture": True,
                 "software_generated_input": False, "latency_acceptance": False, "started_qpc_100ns": base + 40, "completed_qpc_100ns": base + 50}
        files, frames = [], []
        for position, (pixels, offset) in enumerate(((before, 0), (before, 20), (after, 60), (after, 80))):
            path, metrics = self.image(pixels)
            files.append(path)
            frames.append(self.frame(metrics, base + offset, index * 4 + position))
        cursor = [{"before": {"x": -50, "y": -50}, "after": {"x": -50, "y": -50}} for _ in range(4)]
        return {"block": name, "baseline": frames[:2], "after": frames[2:], "phone": phone, "native_phone": native,
                "canvas_files": files, "cursor_samples": cursor}

    def fixture(self, brush_width_follows_pressure=True, barrel_colour=RED, hover_ink=False, eraser_restores=True):
        self.target = {"token": ident(10), "window": 99, "process_id": 100, "thread_id": 101, "process_created": 102,
                       "window_rect": {"x": -5, "y": -20, "width": 2010, "height": 330},
                       "frame_rect": {"x": -5, "y": -20, "width": 2010, "height": 330},
                       "client_rect": {"x": 0, "y": 0, "width": 2000, "height": 300}, "dpi": 96, "integrity": 8192}
        self.image_identity = {"executableName": "mspaint.exe", "executableBlake3": "c" * 64, "executableBytes": 1000, "fileVersion": None,
                               "packageFullName": "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe", "packageVersion": "11.2605.81.0"}
        white = self.pixels()
        pressure_strokes = [stroke(y) for y in PRESSURE_ROWS]
        after_pressure = bytearray(white)
        self.ink(after_pressure, pressure_strokes, BLACK, pressure_width=brush_width_follows_pressure, half=3)
        hover_strokes = [stroke(200, 20, 120)]
        after_hover = bytearray(after_pressure)
        if hover_ink:
            self.ink(after_hover, [[(60, 200, 0.5)]], BLACK, pressure_width=False, half=1)
        barrel_strokes = [stroke(190, 20, 180, 0.5, 0.5)]
        after_barrel = bytearray(after_hover)
        self.ink(after_barrel, barrel_strokes, barrel_colour, pressure_width=False, half=2)
        eraser_strokes = [[(x, 20, 0.5) for x in range(40, 81, 2)]]
        after_eraser = bytearray(after_barrel)
        for x in range(40, 82):
            for y in range(14, 27):
                self.paint(after_eraser, x, y, (255, 255, 255) if eraser_restores else (90, 90, 90))
        blocks = [
            self.block(0, "paint-pressure", white, after_pressure, journal_for("paint-pressure", pressure_strokes)),
            self.block(1, "paint-hover", after_pressure, after_hover, journal_for("paint-hover", hover_strokes, hover_only=True)),
            self.block(2, "paint-barrel", after_hover, after_barrel, journal_for("paint-barrel", barrel_strokes, flags=1, buttons=0x20)),
            self.block(3, "paint-eraser-tool", after_barrel, after_eraser, journal_for("paint-eraser-tool", eraser_strokes)),
        ]
        return {"schema": 1, "target": self.target, "image": self.image_identity, "paint_frame": self.witness(1),
                "eraser_frame": self.witness(3, "Using Eraser tool on Canvas", "9" * 64), "canvas_rect": dict(CANVAS),
                "stroke_radius_pixels": 8, "device_model": "SM-S918U", "blocks": blocks}

    def rows(self, result, tool):
        document = next(d for d in result["documents"] if d["toolId"] == tool)
        return {row["aspect"]: row for row in document["observations"]}

    def test_complete_physical_paint_blocks_produce_valid_rows(self):
        result = reports.report(self.fixture())
        self.assertEqual("passed", result["status"], result["rejection_reason"])
        brush = self.rows(result, "paint.brush")
        self.assertEqual([r["aspect"] for r in result["documents"][0]["observations"]], list(reports.ASPECTS))
        self.assertEqual("Supported", brush["Drawing"]["result"])
        self.assertEqual("Supported", brush["Pressure"]["result"])
        self.assertEqual("Supported", brush["Hover"]["result"])
        self.assertEqual("Supported", brush["BarrelButton"]["result"])
        for aspect in ("Tilt", "InvertedEraser", "Shortcuts"):
            self.assertEqual({"aspect": aspect, "result": "NotTested", "method": "NotPerformed", "inputApi": "Unknown",
                              "evidenceSha256": None, "deviceModel": None}, brush[aspect])
        for aspect in ("Drawing", "Pressure", "Hover", "BarrelButton"):
            row = brush[aspect]
            self.assertEqual(("PhysicalOwnerGesture", "WindowsPointerInput", "SM-S918U"), (row["method"], row["inputApi"], row["deviceModel"]))
            self.assertIn(row["evidenceSha256"], result["evidence"])
            self.assertEqual(row["evidenceSha256"], reports.canonical_sha(result["evidence"][row["evidenceSha256"]]))
        eraser = self.rows(result, "paint.eraser")
        self.assertEqual("Supported", eraser["Drawing"]["result"])
        self.assertEqual("9" * 64, result["documents"][1]["settingsDigest"])
        self.assertEqual("d" * 64, result["documents"][0]["settingsDigest"])
        self.assertTrue(all(eraser[a]["result"] == "NotTested" for a in reports.ASPECTS if a != "Drawing"))
        self.assertTrue(result["editor_effect"])
        self.assertFalse(result["tilt_calibrated"])
        self.assertFalse(result["shortcuts_tested"])
        for document in result["documents"]:
            self.assertEqual(document["identity"]["packageVersion"], "11.2605.81.0")
            self.assertLessEqual(len(reports.document_bytes(document)), 32 * 1024)

    def test_constant_width_with_wide_pressure_is_unsupported(self):
        result = reports.report(self.fixture(brush_width_follows_pressure=False))
        self.assertEqual("passed", result["status"], result["rejection_reason"])
        self.assertEqual("Unsupported", self.rows(result, "paint.brush")["Pressure"]["result"])

    def test_primary_colour_barrel_ink_is_unsupported(self):
        result = reports.report(self.fixture(barrel_colour=BLACK))
        self.assertEqual("Unsupported", self.rows(result, "paint.brush")["BarrelButton"]["result"])

    def test_hover_that_inks_is_unsupported(self):
        result = reports.report(self.fixture(hover_ink=True))
        self.assertEqual("passed", result["status"], result["rejection_reason"])
        self.assertEqual("Unsupported", self.rows(result, "paint.brush")["Hover"]["result"])

    def test_eraser_tool_that_paints_is_unsupported(self):
        result = reports.report(self.fixture(eraser_restores=False))
        self.assertEqual("Unsupported", self.rows(result, "paint.eraser")["Drawing"]["result"])

    def refused(self, value, reason):
        result = reports.report(value)
        self.assertEqual("rejected", result["status"])
        self.assertEqual(reason, result["rejection_reason"])
        self.assertEqual([], result["documents"])

    def test_ink_outside_any_owner_stroke_is_refused(self):
        value = self.fixture()
        strokes = [stroke(y) for y in PRESSURE_ROWS]
        after = self.pixels()
        self.ink(after, strokes, BLACK)
        self.paint(after, 199, 219, BLACK)
        value["blocks"] = [self.block(0, "paint-pressure", self.pixels(), after, journal_for("paint-pressure", strokes))]
        value["eraser_frame"] = None
        self.refused(value, "outside_stroke_region")

    def test_unchained_or_unordered_or_nonwhite_baselines_are_refused(self):
        value = self.fixture()
        value["blocks"][1], value["blocks"][2] = value["blocks"][2], value["blocks"][1]
        self.refused(value, "clock_order")
        value = self.fixture()
        value["blocks"] = value["blocks"][1:2]
        value["eraser_frame"] = None
        self.refused(value, "baseline_not_white")
        value = self.fixture()
        barrel = [stroke(190, 20, 180, 0.5, 0.5)]
        after = self.pixels()
        self.ink(after, barrel, RED, pressure_width=False, half=2)
        value["blocks"][2] = self.block(2, "paint-barrel", self.pixels(), after, journal_for("paint-barrel", barrel, flags=1, buttons=0x20))
        value["blocks"] = value["blocks"][:3]
        value["eraser_frame"] = None
        self.refused(value, "chain_mismatch")

    def test_synthetic_or_unbound_phone_evidence_is_refused(self):
        value = self.fixture()
        value["blocks"][0]["native_phone"]["software_generated_input"] = True
        self.refused(value, "phone_physical")
        value = self.fixture()
        value["blocks"][0]["native_phone"]["controller_input_scenario"] = "balanced"
        self.refused(value, "phone_physical")
        value = self.fixture()
        value["blocks"][0]["phone"]["journal_sha256"] = "0" * 64
        self.refused(value, "phone_physical")
        value = self.fixture()
        Path(value["blocks"][0]["phone"]["journal_path"]).write_bytes(b"{}")
        self.refused(value, "journal_binding")

    def test_eraser_witness_and_census_rules(self):
        value = self.fixture()
        value["eraser_frame"] = None
        self.refused(value, "eraser_witness")
        value = self.fixture()
        value["eraser_frame"]["canvasToolName"] = "Using Brush tool on Canvas"
        self.refused(value, "eraser_witness")
        value = self.fixture()
        value["blocks"].append(copy.deepcopy(value["blocks"][0]))
        self.refused(value, "block_census")
        value = self.fixture()
        value["blocks"][1]["cursor_samples"][1]["after"] = {"x": -49, "y": -50}
        self.refused(value, "cursor_pairs")

    def test_cli_reports_closed_json(self):
        value = self.fixture()
        path = self.root / "physical-evaluation.json"
        path.write_text(json.dumps(value))
        done = subprocess.run([sys.executable, str(HERE / "paint_physical_reports.py"), "--input", str(path)],
                              capture_output=True, text=True, timeout=120, cwd=str(HERE))
        self.assertEqual(0, done.returncode, done.stdout[-400:] + done.stderr[-400:])
        out = json.loads(done.stdout)
        self.assertEqual(("passed", "owned-paint-physical-compatibility"), (out["status"], out["operation"]))
        done = subprocess.run([sys.executable, str(HERE / "paint_physical_reports.py")], capture_output=True, text=True, timeout=60, cwd=str(HERE))
        self.assertEqual("arguments", json.loads(done.stdout)["rejection_reason"])


CROSS_IMAGE = {"executableName": "mspaint.exe", "executableBlake3": "a" * 64, "executableBytes": 1024, "fileVersion": "11.2605.81.0",
               "packageFullName": "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe", "packageVersion": "11.2605.81.0"}


def cross_documents():
    brush = reports.compatibility_document(CROSS_IMAGE, "paint.brush", "d" * 64, {
        "Drawing": ("Supported", "1" * 64), "Pressure": ("Supported", "2" * 64), "Hover": ("Supported", "3" * 64),
        "BarrelButton": ("Inconclusive", "4" * 64)}, "SM-S918U")
    eraser = reports.compatibility_document(CROSS_IMAGE, "paint.eraser", "e" * 64, {"Drawing": ("Supported", "5" * 64)}, "SM-S918U")
    return reports.document_bytes(brush).decode(), reports.document_bytes(eraser).decode()


class KotlinCrossCheckTest(unittest.TestCase):
    """The Kotlin case decodes these exact bytes with NativeRemoteCompatibility."""

    def test_python_documents_equal_the_kotlin_validfor_fixtures(self):
        source = KOTLIN_CASES.read_text(encoding="utf-8")
        fixtures = dict(re.findall(r'/\*(PHYSICAL_[A-Z]+_FIXTURE)\*/"""(.*?)"""', source, re.S))
        brush, eraser = cross_documents()
        self.assertEqual(brush, fixtures["PHYSICAL_BRUSH_FIXTURE"])
        self.assertEqual(eraser, fixtures["PHYSICAL_ERASER_FIXTURE"])


if __name__ == "__main__":
    unittest.main()
