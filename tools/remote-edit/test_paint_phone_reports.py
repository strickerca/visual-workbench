"""Offline adversarial checks of the real Paint phone evidence evaluator."""
import copy
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

from controller_input_reports import ROUTE
from paint_phone_reports import MAX_INPUT, decode_png, paeth, report


def ident(n):
    return f"019f7c21-5678-7000-8000-{n:012x}"


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)


def png(pixels, width, height, mode=0, extra=b"", compressed=None):
    stride = width * 4
    scan = bytearray()
    for y in range(height):
        scan.append(mode)
        for x in range(stride):
            at = y * stride + x
            left = pixels[at - 4] if x >= 4 else 0
            above = pixels[at - stride] if y else 0
            upper_left = pixels[at - stride - 4] if y and x >= 4 else 0
            predicted = (0, left, above, (left + above) // 2, paeth(left, above, upper_left))[mode]
            scan.append((pixels[at] - predicted) & 255)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"sRGB", b"\x00") + extra + chunk(b"IDAT", zlib.compress(scan) if compressed is None else compressed)
            + chunk(b"IEND", b""))


class PaintPhoneReportsTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="paint-phone-evaluator-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.width, self.height = 100, 80
        self.white = b"\xff" * (self.width * self.height * 4)
        dark = bytearray(self.white)
        for x in range(40, 45):
            dark[(40 * self.width + x) * 4:(40 * self.width + x) * 4 + 3] = b"\x00\x00\x00"
        self.dark = bytes(dark)
        self.value = self.fixture()

    def image(self, index, pixels, mode=0):
        path = self.root / f"canvas-{index}.png"
        raw = png(pixels, self.width, self.height, mode)
        path.write_bytes(raw)
        channels = [pixels[i::4] for i in range(4)]
        return str(path), {
            "png_sha256": hashlib.sha256(raw).hexdigest(), "png_blake3": "a" * 64, "png_bytes": len(raw),
            "width": self.width, "height": self.height, "canvas_pixels": self.width * self.height,
            "canvas_offset_x": 0, "canvas_offset_y": 0,
            "rgba_min": [min(c) for c in channels], "rgba_max": [max(c) for c in channels],
            "rgba_sum": [sum(c) for c in channels], "alpha_not_opaque_pixels": sum(a != 255 for a in channels[3]),
            "decoded_rgba_sha256": hashlib.sha256(pixels).hexdigest(), "explicit_srgb": True, "numeric_semantics_proved": False,
        }

    def fixture(self):
        canvas = {"x": 10, "y": 20, "width": self.width, "height": self.height}
        client = {"x": 0, "y": 0, "width": 2000, "height": 150}
        frame_rect = {"x": -5, "y": -20, "width": 2010, "height": 180}
        target = {"token": ident(10), "window": 99, "process_id": 100, "thread_id": 101, "process_created": 102,
                  "window_rect": frame_rect, "frame_rect": frame_rect, "client_rect": client, "dpi": 96, "integrity": 8192}
        image = {"executableName": "mspaint.exe", "executableBlake3": "c" * 64, "executableBytes": 1000,
                 "fileVersion": None, "packageFullName": "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe", "packageVersion": "11.2605.81.0"}
        def witness(start):
            return {"settingsDigest": "d" * 64, "canvasRuntimeIdHash": "e" * 64, "canvasRectHost": copy.deepcopy(canvas),
                    "canvasToolName": "Using Brush tool on Canvas", "observedStartQpc100ns": start, "observedEndQpc100ns": start + 1}
        files, receipts = [], []
        for i, start in enumerate((10, 30, 70, 90)):
            path, metrics = self.image(i, self.white if i < 2 else self.dark)
            files.append(path)
            crop = {"x": 15, "y": 40, "width": self.width, "height": self.height}
            native = {"schema": 1, "sampling_scope": "verified_editor_canvas", "operation": "owned-blank-editor-wgc-metrics", "status": "Measured",
                      "helper_sha256": "f" * 64, "helper_pid": 200 + i, "helper_exit_code": 0,
                      "suspended_before_job_assignment": True, "exact_image_verified_before_resume": True, "process_and_job_retired": True,
                      "io_worker_started": True, "io_join_completed": True, "error": None, "retirement_error": None, "io_error": None,
                      "capture_refusal": None, "capture_color_depth": None, "target": copy.deepcopy(target), "image": copy.deepcopy(image),
                      "source_proof": {"policy": "installed_paint", "held_process": True, "held_final_image": True,
                                       "held_ancestor_namespace": False, "installed_package_namespace": True,
                                       "namespace_lease_equivalence_claim": False, "input_authority": False, "profile_authority": False},
                      "installed_package_evidence": {"full_name": image["packageFullName"], "version": image["packageVersion"], "status_ok": True,
                                                     "status_flags": 0, "signature_kind": 3, "development": False, "framework_or_resource_or_bundle_or_optional": False},
                      "capture_session_id": ident(20), "sample_id": ident(30 + i), "before": witness(start), "after": witness(start + 6),
                      "frame": None, "metrics": None, "diagnostic_sha256": None, "input_sent": False, "profile_authority": False,
                      "numeric_semantics_proved": False, "image_file_presence": True, "root_image_disposal_required": True}
            native["canvas_frame"] = {"source_identity": {"capture_session_id": ident(20), "frame_id": 1, "geometry_revision": 1,
                                                       "source_kind": "window", "client_rect": copy.deepcopy(client), "dpi_scale": 1.0,
                                                       "monotonic_timestamp_ns": (start + 3) * 100, "captured_at_ms": 0, "platform": "windows",
                                                       "window_handle": target["window"], "monitor_id": ""},
                                      "target": {"window": 99, "process_id": 100, "process_created": 102, "client": copy.deepcopy(client),
                                                 "frame": copy.deepcopy(frame_rect), "dpi": 96, "observed_ns": (start + 2) * 100},
                                      "canvas_rect_host": copy.deepcopy(canvas), "crop_in_frame": copy.deepcopy(crop), "source_asset_id": "a" * 64,
                                      "png_bytes": metrics["png_bytes"], "width": self.width, "height": self.height, "bit_depth": 8,
                                      "border_visible": True, "lossless": True, "filename": "capture-canvas.png"}
            native["canvas_metrics"] = {"sampling_scope": "verified_editor_canvas", "image": metrics, "source_client_rect": copy.deepcopy(client),
                                        "canvas_rect_host": copy.deepcopy(canvas), "crop_in_frame": copy.deepcopy(crop)}
            receipts.append(native)
        views = []
        for i in range(3):
            views.append({"remote_render_scope": f"{1 if i < 2 else 2},{ident(50 + i)},1,{ident(11)},1", "remote_render_owner": ident(60 + i),
                          "remote_render_ticket": str(5 + i), "remote_render_frame": str(10 + i), "remote_render_pts_us": str(100000 + i * 1000),
                          "remote_render_codec": "c2.qti.hevc.decoder", "remote_render_timing": f"{900000000 + i * 1000},{910000000 + i * 1000}",
                          "remote_render_capabilities": "hardware=true,low_latency_advertised=false,requested=true,configure_accepted=true"})
        inputs = [{"remote_input_binding": views[0]["remote_render_scope"] + "," + ident(70), "remote_input_sequences": "1,3",
                   "remote_input_owner": views[0]["remote_render_owner"], "remote_input_frame": "12", "remote_input_ticket": "9", "remote_input_pts_us": "120000",
                   "remote_input_timing": "1000000000,1100000000,1110000000,1110000000,1270000000", "remote_input_echo": "3,0,0", "remote_input_route": ROUTE}]
        native_phone = {"schema": 1, "kind": "normal-native-remote-usb-integration", "status": "passed", "model": "SM-S918U", "owner_user": 0,
                        "run_id": "a" * 32, "retained_input_nonce": "a" * 32, "owned_editor_contract_sha256": "b" * 64, "controller_input_scenario": "balanced",
                        "controller_input_records": inputs, "phone_render_records": views, "held_lifecycle_records": []}
        for key in ("controller_input_requested input_grant_attempted input_granted input_attempted input_dispatch_completed software_generated_input "
                    "input_retirement_confirmed host_actual_retirement phone_actual_retirement process_cleanup owned_packages_cleanup private_work_disposed "
                    "external_editor_destination_retained profiles_preserved editor_phone_records_validated phone_render_records_validated").split():
            native_phone[key] = True
        for key in "editor_effect physical_pen_fidelity latency_acceptance controller_input_records_validated held_lifecycle_records_validated".split():
            native_phone[key] = False
        phone = {"schema": 1, "status": "phone-stroke-correlated", "native_receipt_path": "private-native.json", "native_receipt_sha256": "c" * 64,
                 "contract_sha256": "b" * 64, "actual_remote_retirement": True, "retained_parent_returned": True,
                 "receiver_observed": False, "editor_effect": False, "local_history_may_begin": True, "physical_pen_fidelity": False, "latency_acceptance": False,
                 "started_qpc_100ns": 50, "completed_qpc_100ns": 60}
        return {"schema": 1, "target": target, "image": image, "paint_frame": witness(1), "canvas_rect": canvas,
                "baseline": receipts[:2], "after": receipts[2:], "phone": phone, "native_phone": native_phone, "canvas_files": files,
                "stroke_ppm": [25000, 400000, 26000, 400000, 27000, 400000], "stroke_radius_pixels": 4,
                "cursor_samples": [{"before": {"x": -50, "y": -50}, "after": {"x": -50, "y": -50}} for _ in range(4)]}

    def replace_pixels(self, index, pixels, mode=0):
        path, metrics = self.image(index, pixels, mode)
        self.value["canvas_files"][index] = path
        receipt = (self.value["baseline"] + self.value["after"])[index]
        receipt["canvas_metrics"]["image"] = metrics
        receipt["canvas_frame"]["png_bytes"] = metrics["png_bytes"]

    def rejected(self, reason=None):
        result = report(self.value)
        self.assertFalse(result["accepted"], result)
        self.assertFalse(result["editor_effect"])
        self.assertFalse(result["visible_canvas_change"])
        if reason is not None:
            self.assertEqual(reason, result["rejection_reason"])
        return result

    def test_localized_stable_white_to_rgb_change_passes_without_broader_claims(self):
        self.value["native_phone"]["unknown_field"] = "secret-unknown-input"
        result = report(self.value)
        self.assertTrue(result["accepted"], result)
        self.assertNotEqual(self.value["target"]["token"], self.value["native_phone"]["phone_render_records"][0]["remote_render_scope"].split(",")[3])
        self.assertEqual(5, result["pixel_evidence"]["changed_rgb_pixels"])
        self.assertEqual({"x": 40, "y": 40, "width": 5, "height": 1}, result["pixel_evidence"]["changed_bbox_canvas"])
        self.assertEqual(0, result["pixel_evidence"]["outside_stroke_changed_pixels"])
        retained = result["pixel_evidence"]["retained_proof"]
        self.assertEqual(self.value["canvas_rect"], retained["canvas_rect_host"])
        self.assertEqual(self.value["target"]["client_rect"], retained["source_client_rect"])
        self.assertEqual(self.value["paint_frame"]["settingsDigest"], retained["settings_digest"])
        self.assertEqual({"start": 50, "end": 60}, retained["phone_call_qpc_100ns"])
        for observed, native in zip(retained["samples"], self.value["baseline"] + self.value["after"]):
            self.assertEqual(native["sample_id"], observed["sample_id"])
            self.assertEqual(native["capture_session_id"], observed["capture_session_id"])
            self.assertEqual(native["canvas_metrics"]["image"]["png_sha256"], observed["png_sha256"])
            self.assertEqual(native["canvas_metrics"]["image"]["decoded_rgba_sha256"], observed["decoded_rgba_sha256"])
            self.assertEqual(native["canvas_frame"]["source_identity"]["monotonic_timestamp_ns"], observed["capture_monotonic_timestamp_ns"])
        encoded = json.dumps(result)
        for forbidden in (str(self.root), "canvas_files", "native_receipt_path", "secret-unknown-input"):
            self.assertNotIn(forbidden, encoded)
        for key in ("physical_pen_fidelity", "latency_acceptance", "workbench_ui_acceptance"):
            self.assertFalse(result[key])
        self.assertTrue(result["retained_parent_authority_required"])

    def test_no_rgb_change_or_png_reencoding_is_not_a_stroke(self):
        for index in (2, 3):
            self.replace_pixels(index, self.white, index)
        self.rejected("no_rgb_change")

    def test_unstable_before_and_after_pixels_are_rejected(self):
        original = copy.deepcopy(self.value)
        self.replace_pixels(1, self.dark)
        self.rejected("baseline_unstable")
        self.value = original
        self.replace_pixels(1, self.white)
        self.replace_pixels(3, self.white)
        self.rejected("after_unstable")

    def test_opaque_nonwhite_baseline_and_nonopaque_canvas_are_rejected(self):
        for index in (0, 1):
            self.replace_pixels(index, self.dark)
        self.rejected("baseline_not_white")
        for index in (0, 1):
            self.replace_pixels(index, self.white)
        alpha = bytearray(self.dark)
        alpha[3] = 228
        self.replace_pixels(2, bytes(alpha))
        self.rejected("native_pixels")

    def test_target_witness_geometry_and_native_v4_substitution_are_rejected(self):
        original = copy.deepcopy(self.value)
        mutations = [
            lambda r: r["target"].update(process_created=999),
            lambda r: r["after"]["canvasRectHost"].update(x=11),
            lambda r: r["after"].update(settingsDigest="a" * 64),
            lambda r: r["before"].update(canvasRuntimeIdHash="a" * 64),
            lambda r: r.update(sample_id=ident(99).replace("-7000-", "-4000-")),
            lambda r: r["canvas_frame"]["crop_in_frame"].update(x=16),
            lambda r: r["canvas_frame"]["source_identity"].update(window_handle=999),
        ]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.value = copy.deepcopy(original)
                mutation(self.value["after"][0])
                self.rejected()

    def test_full_client_or_forged_metric_cannot_replace_canvas_pixels(self):
        original = copy.deepcopy(self.value)
        self.value["after"][0]["sampling_scope"] = "full_client"
        self.rejected("native_scope")
        self.value = original
        self.value["after"][0]["canvas_metrics"]["image"]["rgba_sum"][0] -= 1
        self.rejected("metrics_mismatch")

    def test_native_error_pending_job_or_unjoined_io_is_rejected(self):
        original = copy.deepcopy(self.value)
        for key, value in (("status", "RetirementPending"), ("process_and_job_retired", False), ("io_join_completed", False),
                           ("error", "Cancelled"), ("capture_refusal", "ColorDepth"), ("helper_exit_code", 1), ("exact_image_verified_before_resume", False)):
            with self.subTest(key=key):
                self.value = copy.deepcopy(original)
                self.value["after"][0][key] = value
                self.rejected()

    def test_missing_phone_grant_actual_retirement_or_balanced_records_is_rejected(self):
        original = copy.deepcopy(self.value)
        for key in ("input_grant_attempted", "input_granted", "input_dispatch_completed", "phone_actual_retirement", "host_actual_retirement", "input_retirement_confirmed"):
            with self.subTest(key=key):
                self.value = copy.deepcopy(original)
                self.value["native_phone"][key] = False
                self.rejected()
        self.value = copy.deepcopy(original)
        self.value["native_phone"]["controller_input_records"][0]["remote_input_echo"] = "2,1,0"
        self.rejected("phone_records")
        self.value = original
        self.value["phone"]["contract_sha256"] = "a" * 64
        self.rejected("phone_binding")

    def test_clock_order_brackets_each_native_capture_and_entire_phone_call(self):
        original = copy.deepcopy(self.value)
        changes = [lambda v: v["after"][0]["canvas_frame"]["source_identity"].update(monotonic_timestamp_ns=7000),
                   lambda v: v["after"][0]["before"].update(observedStartQpc100ns=60),
                   lambda v: v["phone"].update(started_qpc_100ns=37),
                   lambda v: v["phone"].update(completed_qpc_100ns=50),
                   lambda v: v["after"][1]["before"].update(observedStartQpc100ns=77)]
        for change in changes:
            self.value = copy.deepcopy(original)
            change(self.value)
            self.rejected("clock_order")

    def test_changed_pixels_outside_stroke_and_cursor_overlay_are_rejected(self):
        outside = bytearray(self.dark)
        outside[0:3] = b"\x00\x00\x00"
        for index in (2, 3):
            self.replace_pixels(index, bytes(outside))
        self.rejected("outside_stroke_region")
        for index in (2, 3):
            self.replace_pixels(index, self.dark)
        self.value["cursor_samples"][0]["before"] = {"x": 50, "y": 60}
        self.rejected("cursor_inside_canvas")
        self.value["cursor_samples"][0]["before"] = {"x": -50, "y": -50}
        self.value["cursor_samples"][3]["after"]["x"] = -49
        self.rejected("cursor_changed")

    def test_exact_native_file_hash_and_decoder_hash_are_checked(self):
        path = Path(self.value["canvas_files"][2])
        path.write_bytes(png(self.white, self.width, self.height))
        self.rejected("png_hash")
        self.replace_pixels(2, self.dark)
        self.value["after"][0]["canvas_metrics"]["image"]["decoded_rgba_sha256"] = "0" * 64
        self.rejected("decoded_hash")

    def test_quantized_equal_adjacent_phone_points_keep_localized_region(self):
        target = self.value["target"]
        target["client_rect"]["width"] = 640
        target["frame_rect"]["width"] = 650
        target["window_rect"]["width"] = 650
        for frame in self.value["baseline"] + self.value["after"]:
            frame["target"] = copy.deepcopy(target)
            frame["canvas_frame"]["target"]["client"] = copy.deepcopy(target["client_rect"])
            frame["canvas_frame"]["target"]["frame"] = copy.deepcopy(target["frame_rect"])
            frame["canvas_frame"]["source_identity"]["client_rect"] = copy.deepcopy(target["client_rect"])
            frame["canvas_metrics"]["source_client_rect"] = copy.deepcopy(target["client_rect"])
        self.value["stroke_ppm"] = [78125, 400000, 79125, 400000, 80125, 400000]
        dark = bytearray(self.white)
        for x in (40, 41):
            at = (40 * self.width + x) * 4
            dark[at:at + 3] = b"\0\0\0"
        for index in (2, 3):
            self.replace_pixels(index, bytes(dark))
        result = report(self.value)
        self.assertTrue(result["accepted"], result)
        self.assertEqual(2, result["pixel_evidence"]["changed_rgb_pixels"])
        dark[:3] = b"\0\0\0"
        for index in (2, 3):
            self.replace_pixels(index, bytes(dark))
        self.rejected("outside_stroke_region")

    def test_png_all_filters_decode_exactly_and_reject_crc_color_animation_and_bombs(self):
        pixels = bytes((i * 29) % 256 for i in range(3 * 2 * 4))
        for mode in range(5):
            self.assertEqual(pixels, decode_png(png(pixels, 3, 2, mode), 3, 2))
        bad_crc = bytearray(png(pixels, 3, 2)); bad_crc[-1] ^= 1
        invalid = [bytes(bad_crc), png(pixels, 3, 2, extra=chunk(b"iCCP", b"profile")),
                   png(pixels, 3, 2, extra=chunk(b"acTL", b"\0" * 8)),
                   png(pixels, 3, 2, compressed=zlib.compress(b"\0" * 100000)),
                   png(pixels, 3, 2) + b"trailing"]
        for raw in invalid:
            with self.subTest(length=len(raw)), self.assertRaises(ValueError):
                decode_png(raw, 3, 2)

    def test_cli_passes_actual_fixture_and_rejects_duplicate_nonfinite_and_oversized_input(self):
        script = str(Path(__file__).with_name("paint_phone_reports.py"))
        source = self.root / "evidence.json"
        for raw, accepted, reason in ((json.dumps(self.value), True, "none"),
                                      ('{"schema":1,"schema":1}', False, "duplicate_json_key"),
                                      ('{"schema":NaN}', False, "nonfinite_json"),
                                      (" " * (MAX_INPUT + 1), False, "receipt_file")):
            source.write_text(raw, encoding="utf-8")
            process = subprocess.run([sys.executable, script, "--input", str(source)], capture_output=True, text=True, check=False, timeout=20)
            self.assertEqual(0 if accepted else 1, process.returncode, process.stderr)
            output = json.loads(process.stdout)
            self.assertEqual(accepted, output["accepted"])
            self.assertEqual(reason, output["rejection_reason"])
            self.assertLess(len(process.stdout), 8192)
            self.assertNotIn(str(self.root), process.stdout)

    def test_closed_verdict_withholds_unknown_values_and_boolean_integer_substitutions(self):
        self.value["native_phone"]["failure_error_type"] = "secret-private-path"
        result = self.rejected("phone_failure")
        self.assertNotIn("secret", json.dumps(result))
        self.value["native_phone"].pop("failure_error_type")
        self.value["after"][0]["canvas_frame"]["source_identity"]["frame_id"] = True
        self.rejected("native_identity")


if __name__ == "__main__":
    unittest.main()
