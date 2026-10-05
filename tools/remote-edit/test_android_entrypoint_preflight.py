"""Synthetic APK/DEX contract cases. No class loading, SDK, builds or devices."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile
import zlib

import android_entrypoint_preflight as p

CLASS = "Lcom/visualworkbench/android/remote/RemoteNormalPathInstrumentedTest;"
METHOD = "realUsbOwnedWgcHevcControllerRendersBackgroundAndReconnects"
CONTRACT = {"schema": 1, "test_namespace": "Lcom/visualworkbench/android/", "selection": CLASS[1:-1].replace("/", "."), "methods": [METHOD], "runner": "Landroidx/test/ext/junit/runners/AndroidJUnit4;"}


def fixture():
    return {"name": CLASS, "access": 1, "parent": "Ljava/lang/Object;", "annotations": {p.RUN_WITH: (1, {"value": ("type", CONTRACT["runner"])})}, "methods": [
        {"name": "<init>", "descriptor": "()V", "access": 0x10001, "code": True, "annotations": {}},
        {"name": METHOD, "descriptor": "()V", "access": 1, "code": True, "annotations": {p.TEST: (1, {})}},
    ]}


def uleb(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def dex(cls):
    """Small independent serializer for only the JUnit metadata used by fixtures."""
    methods = cls["methods"]
    descriptors = list(dict.fromkeys(m["descriptor"] for m in methods))
    types = [cls["name"], cls["parent"], "V", "I", "Lkotlin/Unit;", p.TEST, p.RUN_WITH, CONTRACT["runner"], p.IGNORE, *p.LIFECYCLE, *p.CLASS_LIFECYCLE]
    types = list(dict.fromkeys(types))
    strings = list(dict.fromkeys([*types, "value", *[m["name"] for m in methods]]))
    si = {s: i for i, s in enumerate(strings)}
    ti = {t: i for i, t in enumerate(types)}
    data = bytearray(112)

    def reserve(size, align=4):
        while len(data) % align:
            data.append(0)
        at = len(data)
        data.extend(bytes(size))
        return at

    def put(at, *values):
        struct.pack_into("<" + "I" * len(values), data, at, *values)

    string_ids = reserve(4 * len(strings))
    type_ids = reserve(4 * len(types))
    proto_ids = reserve(12 * len(descriptors))
    method_ids = reserve(8 * len(methods))
    class_def = reserve(32)
    data_off = len(data)
    for i, text in enumerate(strings):
        at = len(data)
        data.extend(uleb(len(text)) + text.encode() + b"\0")
        put(string_ids + i * 4, at)
    for i, text in enumerate(types):
        put(type_ids + i * 4, si[text])
    for i, descriptor in enumerate(descriptors):
        args, ret = descriptor[1:].split(")")
        off = 0
        if args:
            assert args == "I"
            off = reserve(6)
            put(off, 1)
            struct.pack_into("<H", data, off + 4, ti["I"])
        put(proto_ids + 12 * i, si["V"], ti[ret], off)
    for i, method in enumerate(methods):
        struct.pack_into("<HHI", data, method_ids + 8 * i, ti[cls["name"]], descriptors.index(method["descriptor"]), si[method["name"]])

    def annotations(values):
        offsets = []
        for kind, (visibility, fields) in values.items():
            at = len(data)
            data.append(visibility)
            data.extend(uleb(ti[kind]) + uleb(len(fields)))
            for name, (tag, value) in fields.items():
                assert tag == "type" and ti[value] < 256
                data.extend(uleb(si[name]) + bytes([0x18, ti[value]]))
            offsets.append(at)
        if not offsets:
            return 0
        at = reserve(4 + 4 * len(offsets))
        put(at, len(offsets), *offsets)
        return at

    ca = annotations(cls["annotations"])
    ma = [(i, annotations(m["annotations"])) for i, m in enumerate(methods) if m["annotations"]]
    directory = reserve(16 + 8 * len(ma))
    put(directory, ca, 0, len(ma), 0)
    for i, (method, off) in enumerate(ma):
        put(directory + 16 + i * 8, method, off)
    codes = [reserve(16) if m["code"] else 0 for m in methods]
    class_data = len(data)
    data.extend(uleb(0) + uleb(0) + uleb(1) + uleb(len(methods) - 1))
    for group in [range(1), range(1, len(methods))]:
        previous = 0
        for i in group:
            data.extend(uleb(i - previous) + uleb(methods[i]["access"]) + uleb(codes[i]))
            previous = i
    put(class_def, ti[cls["name"]], cls["access"], ti[cls["parent"]], 0, 0xffffffff, directory, class_data, 0)
    data[:8] = b"dex\n039\0"
    put(32, len(data), 112, 0x12345678)
    for offset, count, at in [(56, len(strings), string_ids), (64, len(types), type_ids), (72, len(descriptors), proto_ids), (88, len(methods), method_ids), (96, 1, class_def)]:
        put(offset, count, at)
    put(104, len(data) - data_off, data_off)
    data[12:32] = hashlib.sha1(data[32:]).digest()
    put(8, zlib.adler32(data[12:]) & 0xffffffff)
    return bytes(data)


class PreflightTests(unittest.TestCase):
    def inspect(self, cls):
        return p.Dex(dex(cls)).inspect(CONTRACT["test_namespace"])

    def check(self, cls):
        return p.check_classes(self.inspect(cls), CONTRACT, CONTRACT["selection"])

    def test_valid_final_apk_binds_actual_bytes_and_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contract = root / "contract.json"
            contract.write_text(json.dumps(CONTRACT), encoding="utf-8")
            apk = root / "test.apk"
            payload = dex(fixture())
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("classes.dex", payload)
            result = p.verify(apk, contract, CONTRACT["selection"])
            self.assertEqual(result["test_count"], 1)
            self.assertEqual(result["apk_sha256"], p.sha(apk.read_bytes()))
            self.assertEqual(result["dex"][0]["sha256"], p.sha(payload))
            self.assertFalse(result["tests_executed"])

    def test_actual_boxed_kotlin_unit_signature_refused(self):
        cls = fixture()
        cls["methods"][1]["descriptor"] = "()Lkotlin/Unit;"
        with self.assertRaisesRegex(ValueError, "junit_method_contract"):
            self.check(cls)

    def test_test_access_parameters_and_body_required(self):
        for field, value in [("access", 0), ("access", 9), ("access", 0x401), ("access", 0x101), ("descriptor", "(I)V"), ("code", False)]:
            with self.subTest(field=field, value=value):
                cls = fixture()
                cls["methods"][1][field] = value
                with self.assertRaises(ValueError):
                    self.check(cls)

    def test_missing_and_duplicate_test_entries_refused(self):
        for change in ("missing", "duplicate", "extra"):
            cls = fixture()
            if change == "missing":
                cls["methods"][1]["annotations"] = {}
            else:
                cls["methods"].append(deepcopy(cls["methods"][1]))
                if change == "extra":
                    cls["methods"][-1]["name"] = "unselectedExtra"
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, "test_census"):
                self.check(cls)

    def test_duplicate_class_across_dex_refused(self):
        classes = self.inspect(fixture())
        with self.assertRaisesRegex(ValueError, "duplicate_class"):
            p.check_classes(classes + classes, CONTRACT, CONTRACT["selection"])

    def test_ignored_class_or_test_refused(self):
        for scope in ("class", "method"):
            cls = fixture()
            (cls if scope == "class" else cls["methods"][1])["annotations"][p.IGNORE] = (1, {})
            with self.subTest(scope=scope), self.assertRaises(ValueError):
                self.check(cls)

    def test_runner_must_be_actual_runtime_android_junit4(self):
        for annotations in ({}, {p.RUN_WITH: (0, {"value": ("type", CONTRACT["runner"])})}, {p.RUN_WITH: (1, {"value": ("type", "Ljava/lang/Object;")})}):
            cls = fixture()
            cls["annotations"] = annotations
            with self.assertRaisesRegex(ValueError, "junit_runner_contract"):
                self.check(cls)

    def test_public_concrete_class_and_zeroarg_constructor_required(self):
        for change in ("private", "abstract", "constructor_private", "constructor_params", "constructor_duplicate"):
            cls = fixture()
            if change == "private": cls["access"] = 0
            if change == "abstract": cls["access"] = 0x401
            if change == "constructor_private": cls["methods"][0]["access"] = 0x10002
            if change == "constructor_params": cls["methods"][0]["descriptor"] = "(I)V"
            if change == "constructor_duplicate": cls["methods"].append(deepcopy(cls["methods"][0]))
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.check(cls)

    def test_local_before_after_reject_nonvoid_parameter_or_static(self):
        for annotation in p.LIFECYCLE:
            for descriptor, access in [("()Lkotlin/Unit;", 1), ("(I)V", 1), ("()V", 9)]:
                cls = fixture()
                cls["methods"].append({"name": "lifecycle", "descriptor": descriptor, "access": access, "code": True, "annotations": {annotation: (1, {})}})
                with self.subTest(annotation=annotation, descriptor=descriptor, access=access), self.assertRaises(ValueError):
                    self.check(cls)

    def test_valid_lifecycle_and_static_class_lifecycle(self):
        cls = fixture()
        for i, annotation in enumerate(sorted(p.LIFECYCLE | p.CLASS_LIFECYCLE)):
            cls["methods"].append({"name": "lifecycle" + str(i), "descriptor": "()V", "access": 9 if annotation in p.CLASS_LIFECYCLE else 1, "code": True, "annotations": {annotation: (1, {})}})
        self.assertEqual(self.check(cls), 1)

    def test_selection_cannot_silently_choose_nothing(self):
        with self.assertRaisesRegex(ValueError, "selection_mismatch"):
            p.check_classes(self.inspect(fixture()), CONTRACT, "wrong.Class")

    def test_truncated_or_tampered_dex_refused(self):
        good = dex(fixture())
        for data in (good[:90], good[:-1], good[:200] + bytes([good[200] ^ 1]) + good[201:]):
            with self.assertRaises(ValueError):
                p.Dex(data)

    def test_dex_table_outside_file_refused_even_with_valid_checksums(self):
        data = bytearray(dex(fixture()))
        struct.pack_into("<I", data, 60, (len(data) + 7) & ~3)
        data[12:32] = hashlib.sha1(data[32:]).digest()
        struct.pack_into("<I", data, 8, zlib.adler32(data[12:]) & 0xffffffff)
        with self.assertRaisesRegex(ValueError, "dex_table_bound"):
            p.Dex(bytes(data))

    def test_apk_missing_or_case_duplicate_dex_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contract = root / "contract.json"
            contract.write_text(json.dumps(CONTRACT), encoding="utf-8")
            for names in [("other.dat",), ("classes.dex", "CLASSES.DEX")]:
                apk = root / "test.apk"
                with zipfile.ZipFile(apk, "w") as archive:
                    for name in names:
                        archive.writestr(name, dex(fixture()))
                with self.subTest(names=names), self.assertRaises(ValueError):
                    p.verify(apk, contract, CONTRACT["selection"])

    def test_contract_duplicate_keys_or_methods_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            contract = Path(directory) / "contract.json"
            duplicate = deepcopy(CONTRACT)
            duplicate["methods"].append(METHOD)
            for text in [json.dumps(duplicate), json.dumps(CONTRACT)[:-1] + ',"schema":1}']:
                contract.write_text(text, encoding="utf-8")
                with self.assertRaises(ValueError):
                    p.load_contract(contract)

    def test_zip_metadata_bound_precedes_zipfile_read(self):
        import io
        value = bytearray(22)
        value[:4] = b"PK\x05\x06"
        struct.pack_into("<I", value, 12, 33 * 1024**2)
        with self.assertRaisesRegex(ValueError, "apk_metadata_bound"):
            p.zip_metadata_bound(io.BytesIO(value), len(value))


if __name__ == "__main__":
    unittest.main()
