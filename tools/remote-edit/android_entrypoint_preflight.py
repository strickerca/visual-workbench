"""Read final APK DEX metadata; never load app classes, install or execute tests.

This is the closed remote-integration JUnit4 contract, not a general DEX verifier.
The existing manifest/signature/native checks remain independently required.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import zipfile
import zlib

TEST = "Lorg/junit/Test;"
IGNORE = "Lorg/junit/Ignore;"
RUN_WITH = "Lorg/junit/runner/RunWith;"
LIFECYCLE = {"Lorg/junit/Before;", "Lorg/junit/After;"}
CLASS_LIFECYCLE = {"Lorg/junit/BeforeClass;", "Lorg/junit/AfterClass;"}
MAX_DEX = 128 * 1024 * 1024


def require(condition, code):
    if not condition:
        raise ValueError(code)


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Dex:
    def __init__(self, data):
        self.data = data
        require(112 <= len(data) <= MAX_DEX, "dex_size")
        require(data[:8] in (b"dex\n035\0", b"dex\n037\0", b"dex\n038\0", b"dex\n039\0", b"dex\n040\0"), "dex_version")
        require(self.u32(32) == len(data) and self.u32(36) == 112 and self.u32(40) == 0x12345678, "dex_header")
        require(data[12:32] == hashlib.sha1(data[32:]).digest(), "dex_signature")
        require(self.u32(8) == zlib.adler32(data[12:]) & 0xffffffff, "dex_checksum")
        self.strings = []
        for at in self.table(56, 4, 500_000):
            pos = self.u32(at)
            units, pos = self.uleb(pos)
            require(units <= 1_048_576, "dex_string_bound")
            end = data.find(b"\0", pos, min(len(data), pos + 3 * units + 1))
            require(end >= pos, "dex_string_end")
            try:
                text = data[pos:end].replace(b"\xc0\x80", b"\0").decode("utf-8", "surrogatepass")
            except UnicodeError as error:
                raise ValueError("dex_string_encoding") from error
            require(len(text.encode("utf-16-le", "surrogatepass")) // 2 == units, "dex_string_length")
            self.strings.append(text)
        self.types = [self.index(self.strings, self.u32(at)) for at in self.table(64, 4, 100_000)]
        self.protos = []
        for at in self.table(72, 12, 100_000):
            ret = self.index(self.types, self.u32(at + 4))
            off = self.u32(at + 8)
            count = self.u32(off) if off else 0
            require(count <= 255, "dex_parameter_count")
            params = [self.index(self.types, self.u16(off + 4 + i * 2)) for i in range(count)]
            self.protos.append("(" + "".join(params) + ")" + ret)
        self.methods = []
        for at in self.table(88, 8, 500_000):
            self.methods.append((self.index(self.types, self.u16(at)), self.index(self.strings, self.u32(at + 4)), self.index(self.protos, self.u16(at + 2))))
        self.classes = list(self.table(96, 32, 100_000))

    @staticmethod
    def index(values, index):
        require(0 <= index < len(values), "dex_index")
        return values[index]

    def u16(self, at):
        require(0 <= at <= len(self.data) - 2, "dex_offset")
        return struct.unpack_from("<H", self.data, at)[0]

    def u32(self, at):
        require(0 <= at <= len(self.data) - 4, "dex_offset")
        return struct.unpack_from("<I", self.data, at)[0]

    def uleb(self, at):
        value = 0
        for shift in range(0, 35, 7):
            require(0 <= at < len(self.data), "dex_uleb_offset")
            byte = self.data[at]
            at += 1
            require(shift != 28 or byte < 16, "dex_uleb_overflow")
            value |= (byte & 127) << shift
            if byte < 128:
                return value, at
        raise ValueError("dex_uleb_overflow")

    def table(self, header, width, maximum):
        count, off = self.u32(header), self.u32(header + 4)
        require(count <= maximum and ((count == 0 and off == 0) or (off >= 112 and off % 4 == 0)), "dex_table")
        require(off + count * width <= len(self.data), "dex_table_bound")
        return range(off, off + count * width, width)

    def annotation(self, at, depth=0):
        require(depth <= 16, "dex_annotation_depth")
        kind, at = self.uleb(at)
        count, at = self.uleb(at)
        require(count <= 1024, "dex_annotation_count")
        values = {}
        for _ in range(count):
            name, at = self.uleb(at)
            name = self.index(self.strings, name)
            require(name not in values, "dex_annotation_duplicate")
            values[name], at = self.value(at, depth + 1)
        return (self.index(self.types, kind), values), at

    def value(self, at, depth):
        require(depth <= 16 and 0 <= at < len(self.data), "dex_value_depth")
        byte = self.data[at]
        kind, arg, at = byte & 31, byte >> 5, at + 1
        if kind == 0x1c:
            require(arg == 0, "dex_array_arg")
            count, at = self.uleb(at)
            require(count <= 4096, "dex_array_bound")
            for _ in range(count):
                _, at = self.value(at, depth + 1)
            return None, at
        if kind == 0x1d:
            require(arg == 0, "dex_annotation_arg")
            return self.annotation(at, depth + 1)
        if kind in (0x1e, 0x1f):
            require(arg == 0 if kind == 0x1e else arg <= 1, "dex_null_boolean")
            return None, at
        widths = {0: 0, 2: 1, 3: 1, 4: 3, 6: 7, 0x10: 3, 0x11: 7, 0x15: 3, 0x16: 3, 0x17: 3, 0x18: 3, 0x19: 3, 0x1a: 3, 0x1b: 3}
        require(kind in widths and arg <= widths[kind] and at + arg + 1 <= len(self.data), "dex_value_kind")
        number = int.from_bytes(self.data[at:at + arg + 1], "little")
        return (("type", self.index(self.types, number)) if kind == 0x18 else None), at + arg + 1

    def annotation_set(self, off):
        if not off:
            return {}
        count = self.u32(off)
        require(count <= 256, "dex_annotation_set_bound")
        result = {}
        for i in range(count):
            at = self.u32(off + 4 + 4 * i)
            require(at < len(self.data) and self.data[at] <= 2, "dex_annotation_visibility")
            (kind, value), _ = self.annotation(at + 1)
            require(kind not in result, "dex_annotation_kind_duplicate")
            result[kind] = (self.data[at], value)
        return result

    def definitions(self, at):
        if not at:
            return {}
        counts = []
        for _ in range(4):
            count, at = self.uleb(at)
            require(count <= 100_000, "dex_class_data_count")
            counts.append(count)
        for count in counts[:2]:
            for _ in range(count):
                _, at = self.uleb(at)
                _, at = self.uleb(at)
        methods = {}
        for count in counts[2:]:
            index = 0
            for i in range(count):
                delta, at = self.uleb(at)
                require(i == 0 or delta > 0, "dex_method_order")
                index += delta
                access, at = self.uleb(at)
                code, at = self.uleb(at)
                require(index not in methods and index < len(self.methods), "dex_method_duplicate")
                require(code == 0 or 0 < code < len(self.data), "dex_code_offset")
                methods[index] = (access, code)
        return methods

    def inspect(self, namespace):
        result = []
        for at in self.classes:
            name = self.index(self.types, self.u32(at))
            if not name.startswith(namespace):
                continue
            directory = self.u32(at + 20)
            class_annotations, method_annotations = {}, {}
            if directory:
                class_annotations = self.annotation_set(self.u32(directory))
                fields, count, params = (self.u32(directory + n) for n in (4, 8, 12))
                require(fields + count + params <= 100_000, "dex_directory_bound")
                require(directory + 16 + 8 * (fields + count + params) <= len(self.data), "dex_directory_offset")
                for i in range(count):
                    slot = directory + 16 + 8 * (fields + i)
                    method = self.u32(slot)
                    require(method not in method_annotations, "dex_method_annotation_duplicate")
                    method_annotations[method] = self.annotation_set(self.u32(slot + 4))
            definitions = self.definitions(self.u32(at + 24))
            require(set(method_annotations) <= set(definitions), "dex_annotation_undefined_method")
            methods = []
            for index, (access, code) in definitions.items():
                owner, method, descriptor = self.methods[index]
                require(owner == name, "dex_method_owner")
                methods.append({"name": method, "descriptor": descriptor, "access": access, "code": bool(code), "annotations": method_annotations.get(index, {})})
            parent = self.u32(at + 8)
            result.append({"name": name, "access": self.u32(at + 4), "parent": self.index(self.types, parent) if parent != 0xffffffff else None, "annotations": class_annotations, "methods": methods})
        return result


def load_contract(path):
    raw = Path(path).read_bytes()
    require(0 < len(raw) <= 16_384, "contract_size")
    def closed(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "contract_duplicate")
            result[key] = value
        return result
    contract = json.loads(raw, object_pairs_hook=closed)
    require(set(contract) == {"schema", "test_namespace", "selection", "methods", "runner"} and type(contract["schema"]) is int and contract["schema"] == 1, "contract_schema")
    require(re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$.]+", contract["selection"]) is not None, "contract_selection")
    require(contract["test_namespace"] == "Lcom/visualworkbench/android/", "contract_namespace")
    require(contract["runner"] == "Landroidx/test/ext/junit/runners/AndroidJUnit4;", "contract_runner")
    methods = contract["methods"]
    require(isinstance(methods, list) and 1 <= len(methods) <= 32 and all(isinstance(m, str) and re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$]*", m) for m in methods) and len(set(methods)) == len(methods), "contract_methods")
    return contract, sha(raw)


def check_classes(classes, contract, selection):
    require(selection == contract["selection"], "selection_mismatch")
    selected = "L" + selection.replace(".", "/") + ";"
    names = [c["name"] for c in classes]
    require(len(set(names)) == len(names), "duplicate_class")
    tests = [(c, m) for c in classes for m in c["methods"] if TEST in m["annotations"]]
    require(len(tests) == len(contract["methods"]) and {(c["name"], m["name"]) for c, m in tests} == {(selected, m) for m in contract["methods"]}, "test_census")
    selected_classes = [c for c in classes if c["name"] == selected]
    require(len(selected_classes) == 1, "selected_class_missing")
    cls = selected_classes[0]
    require(cls["access"] & 1 and not cls["access"] & (0x200 | 0x400) and cls["parent"] == "Ljava/lang/Object;", "junit_class_contract")
    require(IGNORE not in cls["annotations"], "junit_ignored_class")
    require(cls["annotations"].get(RUN_WITH) == (1, {"value": ("type", contract["runner"])}), "junit_runner_contract")
    constructors = [m for m in cls["methods"] if m["name"] == "<init>" and m["access"] & 1]
    require(len(constructors) == 1 and constructors[0]["descriptor"] == "()V" and not constructors[0]["access"] & (8 | 0x100 | 0x400) and constructors[0]["code"], "junit_constructor_contract")
    for method in cls["methods"]:
        annotations = method["annotations"]
        applicable = set(annotations) & ({TEST} | LIFECYCLE | CLASS_LIFECYCLE)
        if not applicable:
            continue
        require(len(applicable) == 1 and IGNORE not in annotations, "junit_method_annotations")
        require(all(annotations[a][0] == 1 for a in applicable), "junit_annotation_visibility")
        require(method["descriptor"] == "()V" and method["access"] & 1 and not method["access"] & (0x100 | 0x400) and method["code"], "junit_method_contract")
        require(bool(method["access"] & 8) == bool(applicable & CLASS_LIFECYCLE), "junit_method_static")
    return len(tests)


def zip_metadata_bound(stream, size):
    # Bound metadata before ZipFile materializes its central-directory entries.
    stream.seek(max(0, size - 65_557))
    tail = stream.read(65_557)
    candidates = []
    start = 0
    while True:
        at = tail.find(b"PK\x05\x06", start)
        if at < 0:
            break
        if at + 22 <= len(tail) and at + 22 + struct.unpack_from("<H", tail, at + 20)[0] == len(tail):
            candidates.append(at)
        start = at + 1
    require(len(candidates) == 1, "apk_zip_end")
    at = candidates[0]
    disk, directory_disk, disk_count, count, directory_size, offset = struct.unpack_from("<4H2I", tail, at + 4)
    require(disk == directory_disk == 0 and disk_count == count and count <= 65_534, "apk_zip_layout")
    require(directory_size <= 32 * 1024**2 and offset + directory_size <= size - len(tail) + at, "apk_metadata_bound")
    require(at < 20 or tail[at - 20:at - 16] != b"PK\x06\x07", "apk_zip64")
    stream.seek(0)


def verify(apk, contract_path, selection):
    contract, contract_hash = load_contract(contract_path)
    path = Path(apk)
    require(path.is_file() and 0 < path.stat().st_size <= 2 * 1024**3, "apk_bound")
    # Hash the same retained file descriptor used for ZIP inspection.
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
        zip_metadata_bound(stream, path.stat().st_size)
        with zipfile.ZipFile(stream) as archive:
            entries = archive.infolist()
            require(len(entries) <= 65_534, "apk_entry_bound")
            names = [e.filename for e in entries]
            require(len(set(n.casefold() for n in names)) == len(names), "apk_duplicate_entry")
            dex = [e for e in entries if re.fullmatch(r"classes(?:[2-9]|[1-9][0-9]+)?\.dex", e.filename)]
            require(1 <= len(dex) <= 32 and "classes.dex" in [e.filename for e in dex], "apk_dex_census")
            require(sum(e.file_size for e in dex) <= 256 * 1024**2, "apk_dex_total")
            classes, bindings = [], []
            for entry in dex:
                require(not entry.flag_bits & 1 and 112 <= entry.file_size <= MAX_DEX, "apk_dex_bound")
                data = archive.read(entry)
                require(len(data) == entry.file_size, "apk_dex_length")
                classes.extend(Dex(data).inspect(contract["test_namespace"]))
                bindings.append({"entry": entry.filename, "sha256": sha(data)})
    count = check_classes(classes, contract, selection)
    return {"schema": 1, "status": "passed", "apk_sha256": digest, "contract_sha256": contract_hash, "selection": selection, "test_count": count, "methods": contract["methods"], "dex": bindings, "tests_executed": False, "manifest_signature_native_checks_required": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apk-test", required=True)
    parser.add_argument("--contract", required=True)
    parser.add_argument("--selection", required=True)
    args = parser.parse_args()
    try:
        result = verify(args.apk_test, args.contract, args.selection)
    except (ValueError, OSError, zipfile.BadZipFile, UnicodeError, TypeError, KeyError) as error:
        code = str(error) if isinstance(error, ValueError) and re.fullmatch(r"[a-z_]+", str(error)) else "preflight_invalid_input"
        print(json.dumps({"schema": 1, "status": "refused", "reason": code, "tests_executed": False}))
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
