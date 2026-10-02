"""Synthetic parser regressions, with no live device identifiers or UI use."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(shutil.which("powershell.exe"), "Windows PowerShell required")
class DiagnosticParserTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        environment = {key: value for key, value in os.environ.items()
                       if not re.search(r"TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY", key, re.I)}
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(ROOT / "tools/tests/diagnostic-parser-fixtures.ps1")],
            # PowerShell startup can exceed 15 seconds under the bounded Gradle
            # build's load. The enclosing test runner emits progress and owns the
            # complete process tree; this remains a separate finite fixture limit.
            capture_output=True, text=True, env=environment, timeout=60, check=True)
        cls.output = result.stdout
        cls.data = json.loads(result.stdout)

    def test_touch_pressure_is_not_classified_as_pen(self):
        self.assertEqual(self.data["touch_only"], [])
        self.assertEqual(len(self.data["pen"]), 1)

    def test_axes_are_bound_to_the_pen_device(self):
        axes = self.data["pen"][0]["axes"]
        self.assertFalse(axes["ABS_TILT_X"]["present"])
        self.assertTrue(axes["ABS_TILT_Y"]["present"])
        self.assertTrue(axes["ABS_DISTANCE"]["present"])
        self.assertEqual(axes["ABS_PRESSURE"]["maximum"], 4095)

    def test_display_modes_are_numeric_and_deduplicated(self):
        self.assertEqual(len(self.data["display"]), 2)
        self.assertEqual({row["refresh_hz"] for row in self.data["display"]}, {60, 90})

    def test_codec_names_are_allowlisted_and_deduplicated(self):
        self.assertEqual(self.data["codec"], ["OMX.qcom.video.decoder.avc"])

    def test_raw_input_identifiers_are_omitted(self):
        self.assertNotIn("private", self.output)
        self.assertNotIn("/dev/input", self.output)
