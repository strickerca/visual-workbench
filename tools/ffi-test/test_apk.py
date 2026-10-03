#!/usr/bin/env python3
"""Synthetic, standard-library-only tests; no real APKs, builds or devices.

Run centrally with: python tools/ffi-test/test_apk.py
"""

from __future__ import annotations

from dataclasses import replace
import hashlib
import io
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock
import zlib

import check_apk as verifier


CORE = "lib/arm64-v8a/libvw_core.so"
JNA = "lib/arm64-v8a/libjnidispatch.so"
CAMERA = "lib/arm64-v8a/libimage_processing_util_jni.so"


def elf(*, alignment: int = 16_384, address: int = 0, file_size: int = 4096,
        memory_size: int | None = None, offset: int = 0, loads: bool = True) -> bytes:
    value = bytearray(file_size)
    identity = b"\x7fELF\x02\x01\x01" + b"\0" * 9
    struct.pack_into("<16sHHIQQQIHHHHHH", value, 0, identity, 3, 183, 1,
                     0, 64, 0, 0, 64, 56, 1, 0, 0, 0)
    struct.pack_into("<II6Q", value, 64, 1 if loads else 0, 5, offset, address,
                     address, file_size - offset, memory_size if memory_size is not None else file_size,
                     alignment)
    # Fixed recognizable contents independent of the verifier's checks/hash code.
    value[128:144] = b"synthetic-native"
    return bytes(value)


def changed(data: bytes, offset: int, fmt: str, value: int) -> bytes:
    result = bytearray(data)
    struct.pack_into(fmt, result, offset, value)
    return bytes(result)


def apk(entries: list[tuple[str, bytes, dict[str, object]]] | None = None,
        *, gap: bytes = b"", comment: bytes = b"") -> bytes:
    if entries is None:
        entries = [(CORE, elf(), {}), (JNA, elf(), {})]
    body = bytearray()
    central = bytearray()
    for name, data, options in entries:
        raw = name.encode("utf-8")
        local_name = str(options.get("local_name", name)).encode("utf-8")
        flags = int(options.get("flags", 0x800))
        method = int(options.get("method", 0))
        compressed = data
        if method == 8:
            compressor = zlib.compressobj(wbits=-15)
            compressed = compressor.compress(data) + compressor.flush()
        crc = zlib.crc32(data)
        local_at = len(body)
        align = int(options.get("zip_alignment", 16_384))
        padding = (-(local_at + 30 + len(local_name))) % align if align else 0
        extra = b"\0" * padding
        descriptor = bool(flags & 8)
        local_crc = 0 if descriptor else crc
        local_sizes = (0, 0) if descriptor else (len(compressed), len(data))
        body += struct.pack("<4s5H3I2H", b"PK\x03\x04", 20, flags, method, 0, 0,
                            local_crc, *local_sizes, len(local_name), len(extra))
        body += local_name + extra + compressed
        if descriptor:
            if options.get("descriptor_signature", True):
                body += b"PK\x07\x08"
            body += struct.pack("<3I", crc, len(compressed), len(data))
        external = int(options.get("external", 0o100644 << 16))
        central += struct.pack("<4s6H3I5H2I", b"PK\x01\x02", 0x314, 20,
                               flags, method, 0, 0, crc, len(compressed), len(data),
                               len(raw), 0, 0, 0, 0, external, int(options.get("offset", local_at)))
        central += raw
    body += gap
    at = len(body)
    body += central
    body += struct.pack("<4s4H2IH", b"PK\x05\x06", 0, 0, len(entries), len(entries),
                        len(central), at, len(comment)) + comment
    return bytes(body)


class ApkInventoryTest(unittest.TestCase):
    def inspect(self, value: bytes, **kwargs: object) -> dict[str, object]:
        with tempfile.TemporaryDirectory(prefix="vw-apk-test-") as folder:
            path = Path(folder) / "synthetic.apk"
            path.write_bytes(value)
            return verifier.verify_apk(path, **kwargs)

    def refuses(self, value: bytes, code: str, **kwargs: object) -> None:
        with self.assertRaisesRegex(verifier.Rejected, "^" + code + "$"):
            self.inspect(value, **kwargs)

    def test_every_native_is_hashed_including_unlisted_transitive_library(self) -> None:
        camera = elf(address=16_384)
        value = apk([(CORE, elf(), {}), (JNA, elf(), {}), (CAMERA, camera, {}),
                     ("classes.dex", b"synthetic-resource", {"method": 8, "zip_alignment": 0})],
                    gap=b"synthetic APK signing block (not authenticated)")
        result = self.inspect(value)
        self.assertEqual(result["apk_sha256"], hashlib.sha256(value).hexdigest())
        self.assertEqual(result["apk_bytes"], len(value))
        self.assertFalse(result["runtime_16k_verified"])
        self.assertFalse(result["signature_verified"])
        self.assertEqual(result["extracted_files"], 0)
        libraries = {item["entry"]: item for item in result["libraries"]}
        self.assertEqual(set(libraries), {CORE, JNA, CAMERA})
        self.assertEqual(libraries[CAMERA]["sha256"], hashlib.sha256(camera).hexdigest())
        self.assertEqual(libraries[CAMERA]["loads"][0]["virtual_address"], 16_384)
        self.assertTrue(all(item["zip_data_offset"] % 16_384 == 0 for item in libraries.values()))

    def test_both_descriptor_forms_and_embedded_eocd_signature_in_comment(self) -> None:
        for signature in (True, False):
            with self.subTest(signature=signature):
                value = apk([(CORE, elf(), {"flags": 0x808, "descriptor_signature": signature}),
                             (JNA, elf(), {})], comment=b"comment PK\x05\x06 invalid suffix")
                self.assertEqual(self.inspect(value)["status"], "PASS")

    def test_required_libraries_cannot_be_replaced_by_additional_requirements(self) -> None:
        self.refuses(apk([(CORE, elf(), {})]), "required_library_missing")
        self.refuses(apk(), "required_library_missing", additional_required=("libcamera.so",))
        self.refuses(apk(), "invalid_required_library", additional_required=("../libextra.so",))

    def test_unexpected_abi_and_misplaced_native_payload_are_refused(self) -> None:
        for name, code in [("lib/x86_64/libother.so", "unexpected_abi"),
                           ("assets/libother.so", "unexpected_native_path"),
                           ("lib/arm64-v8a/not-native.txt", "unexpected_native_path")]:
            with self.subTest(name=name):
                self.refuses(apk([(CORE, elf(), {}), (JNA, elf(), {}), (name, elf(), {})]), code)

    def test_duplicates_encryption_symlinks_and_unsafe_names_are_refused(self) -> None:
        self.refuses(apk([(CORE, elf(), {}), (JNA, elf(), {}), (CORE, elf(), {})]), "duplicate_entry")
        for flags in (0x801, 0x840):
            self.refuses(apk([(CORE, elf(), {"flags": flags}), (JNA, elf(), {})]), "encrypted_or_unsupported_flags")
        self.refuses(apk([(CORE, elf(), {"external": 0o120777 << 16}), (JNA, elf(), {})]), "nonregular_entry")
        for name in ("../secret", "/absolute", "a\\b", "a/./b", "a\0b"):
            with self.subTest(name=name):
                self.refuses(apk([(CORE, elf(), {}), (JNA, elf(), {}), (name, b"", {})]), "unsafe_entry_name")

    def test_compressed_native_and_four_kib_zip_alignment_are_refused(self) -> None:
        self.refuses(apk([(CORE, elf(), {"method": 8}), (JNA, elf(), {})]), "compressed_native_library")
        self.refuses(apk([(CORE, elf(), {"zip_alignment": 4096}), (JNA, elf(), {})]), "zip_native_alignment")

    def test_local_name_and_size_disagreement_are_refused(self) -> None:
        self.refuses(apk([(CORE, elf(), {"local_name": CORE.replace("core", "xxxx")}), (JNA, elf(), {})]), "local_name_mismatch")
        self.refuses(changed(apk(), 18, "<I", 1), "local_size_crc_mismatch")

    def test_native_corruption_is_detected_after_streaming_hash(self) -> None:
        value = bytearray(apk())
        value[16_384 + 200] ^= 1
        self.refuses(bytes(value), "native_crc_mismatch")

    def test_descriptor_corruption_and_overlapping_local_ranges_are_refused(self) -> None:
        descriptor = apk([(CORE, elf(), {"flags": 0x808}), (JNA, elf(), {})])
        self.refuses(changed(descriptor, 16_384 + 4096 + 4, "<I", 1), "data_descriptor_mismatch")
        value = apk([("resource", b"r", {"zip_alignment": 0}), (CORE, elf(), {}), (JNA, elf(), {})])
        cd_at = struct.unpack_from("<I", value, len(value) - 6)[0]
        # Give the ordinary resource a locally/centrally consistent range that
        # covers the following native entry. No file is extracted or inflated.
        length = 16_384 + 4096 - (30 + len("resource"))
        for at in (18, 22, cd_at + 20, cd_at + 24):
            value = changed(value, at, "<I", length)
        self.refuses(value, "overlapping_zip_entries")

    def test_truncated_and_out_of_range_directory_are_refused(self) -> None:
        self.refuses(apk()[:-1], "missing_eocd")
        value = apk()
        self.refuses(changed(value, len(value) - 6, "<I", len(value)), "central_directory_range")
        self.refuses(changed(value, len(value) - 12, "<H", 0xFFFF), "multidisk_zip")

    def test_zip64_and_multidisk_are_refused_before_entry_parsing(self) -> None:
        value = apk()
        self.refuses(changed(value, len(value) - 6, "<I", 0xFFFFFFFF), "zip64_unsupported")
        self.refuses(changed(value, len(value) - 18, "<H", 1), "multidisk_zip")

    def test_elf_identity_must_be_arm64_little_endian_shared_object(self) -> None:
        for at, fmt, value in [(4, "<B", 1), (5, "<B", 2), (16, "<H", 2),
                               (18, "<H", 62), (20, "<I", 2), (52, "<H", 63)]:
            with self.subTest(offset=at):
                self.refuses(apk([(CORE, changed(elf(), at, fmt, value), {}), (JNA, elf(), {})]), "elf_arm64_shared_header")

    def test_elf_program_table_is_counted_and_bounded_before_reads(self) -> None:
        for at, fmt, value in [(32, "<Q", 2**63), (32, "<Q", 8), (32, "<Q", 65),
                               (54, "<H", 0), (56, "<H", 0), (56, "<H", 129), (56, "<H", 0xFFFF)]:
            with self.subTest(offset=at, value=value):
                self.refuses(apk([(CORE, changed(elf(), at, fmt, value), {}), (JNA, elf(), {})]), "elf_program_header_bounds")

    def test_load_alignment_and_offset_address_congruence(self) -> None:
        for align in (0, 1, 4096, 16_385):
            with self.subTest(alignment=align):
                self.refuses(apk([(CORE, elf(alignment=align), {}), (JNA, elf(), {})]), "elf_load_alignment")
        self.refuses(apk([(CORE, elf(address=1), {}), (JNA, elf(), {})]), "elf_load_congruence")
        self.refuses(apk([(CORE, elf(offset=1), {}), (JNA, elf(), {})]), "elf_load_congruence")
        self.assertEqual(self.inspect(apk([(CORE, elf(alignment=65_536), {}), (JNA, elf(), {})]))["status"], "PASS")

    def test_load_file_memory_ranges_and_presence(self) -> None:
        self.refuses(apk([(CORE, changed(elf(), 64 + 32, "<Q", 4097), {}), (JNA, elf(), {})]), "elf_segment_file_range")
        self.refuses(apk([(CORE, elf(memory_size=1), {}), (JNA, elf(), {})]), "elf_load_range")
        self.refuses(apk([(CORE, elf(address=2**64 - 16_384, memory_size=16_384), {}), (JNA, elf(), {})]), "elf_load_range")
        self.refuses(apk([(CORE, elf(loads=False), {}), (JNA, elf(), {})]), "elf_missing_load")
        self.refuses(apk([(CORE, elf(memory_size=2 * 1024**3), {}), (JNA, elf(), {})]), "elf_load_memory_limit")

    def test_explicit_inventory_metadata_and_file_limits(self) -> None:
        for field, value, code in [("max_apk_bytes", 100, "apk_size_limit"),
                                   ("max_entries", 1, "entry_limit"),
                                   ("max_libraries", 1, "native_inventory_limit"),
                                   ("max_metadata_bytes", 64, "metadata_limit"),
                                   ("max_name_bytes", 8, "entry_name_limit"),
                                   ("max_library_bytes", 128, "library_size_limit"),
                                   ("max_load_memory_bytes", 128, "elf_load_memory_limit")]:
            with self.subTest(field=field):
                self.refuses(apk(), code, limits=replace(verifier.Limits(), **{field: value}))
        self.refuses(apk(), "local_metadata_limit", limits=replace(verifier.Limits(), max_metadata_bytes=1000))

    def test_hashing_is_bounded_and_never_reads_the_whole_region(self) -> None:
        value = bytes(range(256)) * (verifier.CHUNK // 256 + 17)
        class BoundedReader(io.BytesIO):
            def read(self, size: int = -1) -> bytes:
                self_test.assertGreater(size, 0)
                self_test.assertLessEqual(size, verifier.CHUNK)
                return super().read(size)
        self_test = self
        digest, crc = verifier.hash_region(BoundedReader(value), 0, len(value))
        self.assertEqual(digest, hashlib.sha256(value).hexdigest())
        self.assertEqual(crc, zlib.crc32(value))

    def test_library_program_header_reads_do_not_buffer_the_library(self) -> None:
        value = elf(file_size=3 * verifier.CHUNK)
        sizes = []
        class HeaderReader(io.BytesIO):
            def read(self, size: int = -1) -> bytes:
                sizes.append(size)
                return super().read(size)
        loads = verifier.elf_headers(HeaderReader(value), 0, len(value), verifier.Limits())
        self.assertEqual(sizes, [64, 56])
        self.assertEqual(loads[0]["file_bytes"], len(value))

    def test_refusal_receipt_contains_no_input_path(self) -> None:
        with mock.patch("sys.argv", ["check_apk.py", "private-missing-input.apk"]), mock.patch("sys.stdout", new_callable=io.StringIO) as output:
            self.assertEqual(verifier.main(), 1)
        self.assertNotIn("private-missing-input", output.getvalue())
        self.assertIn('"runtime_16k_verified": false', output.getvalue())


if __name__ == "__main__":
    unittest.main()
