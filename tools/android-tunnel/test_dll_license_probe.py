"""Tests the future DLL diagnostic with fake functions; no Windows DLL is loaded."""

import ctypes
import importlib.util
from pathlib import Path
import shlex
import sys
import unittest


sys.path.insert(0, str(Path(__file__).parent))
SPEC = importlib.util.spec_from_file_location("dll_license_probe", Path(__file__).with_name("dll_license_probe.py"))
assert SPEC and SPEC.loader
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def release_configuration() -> str:
    options = ["--prefix=/build/app/deps/work/install/win64-cross-shared",
               *sorted(probe.EXPECTED_CONFIGURATION)]
    return shlex.join(options)


class FakeFunction:
    def __init__(self, value: bytes):
        self.buffer = ctypes.create_string_buffer(value + b"\0")
        self.restype = None
        self.argtypes = None

    def __call__(self):
        if self.restype is not ctypes.c_void_p or self.argtypes != []:
            raise AssertionError("The pointer must remain unconverted until after the size cap")
        return ctypes.addressof(self.buffer)


class DllLicenseProbeTest(unittest.TestCase):
    def test_exact_v41_recipe_and_pointer_bounded_strings(self):
        license_function = FakeFunction(probe.EXPECTED_LICENSE.encode("ascii"))
        configuration_function = FakeFunction(release_configuration().encode("ascii"))
        library = type("FakeLibrary", (), {
            "avcodec_license": license_function,
            "avcodec_configuration": configuration_function,
        })()
        license_text, configuration = probe._query_ffmpeg(library, "avcodec")
        probe._validate_ffmpeg_claim(license_text, configuration)
        self.assertEqual(probe.EXPECTED_LICENSE, license_text)
        self.assertEqual(release_configuration(), configuration)
        self.assertIs(ctypes.c_void_p, license_function.restype)
        self.assertIs(ctypes.c_void_p, configuration_function.restype)

    def test_empty_misleading_gpl_and_unknown_configuration_refuse(self):
        valid = release_configuration()
        wrong = [
            ("LGPL version 3 or later", valid),
            ("not LGPL version 2.1 or later", valid),
            ("GPL version 2 or later; LGPL compatible", valid),
            (probe.EXPECTED_LICENSE, ""),
            (probe.EXPECTED_LICENSE, "--enable-shared"),
            (probe.EXPECTED_LICENSE, valid + " --enable-gpl"),
            (probe.EXPECTED_LICENSE, valid + " --enable-nonfree"),
            (probe.EXPECTED_LICENSE, valid + " --enable-libx264"),
            (probe.EXPECTED_LICENSE, valid + " --disable-static"),
            (probe.EXPECTED_LICENSE, valid.replace("--arch=x86_64", "--arch=x86")),
            (probe.EXPECTED_LICENSE, valid.replace("--enable-shared", "--disable-shared")),
            (probe.EXPECTED_LICENSE, valid + " '"),
        ]
        for license_text, configuration in wrong:
            with self.subTest(license=license_text, configuration=configuration[-40:]):
                with self.assertRaises(probe.Refusal):
                    probe._validate_ffmpeg_claim(license_text, configuration)

    def test_null_empty_nonascii_and_oversized_pointers_refuse_before_copy(self):
        for raw in (b"", b"\xff", b"A" * (probe.MAX_LICENSE_BYTES + 1)):
            with self.subTest(length=len(raw)):
                buffer = ctypes.create_string_buffer(raw + b"\0")
                with self.assertRaises(probe.Refusal):
                    probe._bounded_c_text(ctypes.addressof(buffer), probe.MAX_LICENSE_BYTES, "license")
        with self.assertRaises(probe.Refusal):
            probe._bounded_c_text(None, probe.MAX_LICENSE_BYTES, "license")
        huge = ctypes.create_string_buffer(b"x" * (probe.MAX_CONFIGURATION_BYTES + 1) + b"\0")
        with self.assertRaises(probe.Refusal):
            probe._bounded_c_text(ctypes.addressof(huge), probe.MAX_CONFIGURATION_BYTES, "configuration")


if __name__ == "__main__":
    unittest.main()
