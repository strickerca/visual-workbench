#!/usr/bin/env python3
"""Bounded, standard-library-only inspection; this never loads or runs a model."""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import stat
import struct
import sys

MAX_JSON = 1_048_576
MAX_ARTIFACT = 134_217_728
MAX_TOTAL = 268_435_456
MAX_TENSORS = 32_768
MAX_OPERATORS = 8_192
MAX_METADATA_READS = 2_000_000
GRAPH_CONTRACT = "mobilesam-vit-t-fixed66-masked-v1"
PREPROCESSING = {"schema": "sam-rgb-resize-longest1024-v1"}
POSTPROCESSING = {"schema": "sam-mask0-logits-inverse-v1"}
CONTRACTS = {
    "encoder": {
        "inputs": [("image", "float32", [1, 3, 1024, 1024])],
        "outputs": [("image_embeddings", "float32", [1, 256, 64, 64])],
    },
    "prompt_decoder": {
        "inputs": [
            ("image_embeddings", "float32", [1, 256, 64, 64]),
            ("point_coords", "float32", [1, 66, 2]),
            ("point_labels", "int32", [1, 66]),
            ("point_valid", "int32", [1, 66]),
            ("token_count", "int32", [1]),
        ],
        "outputs": [
            ("mask_logits", "float32", [1, 1, 256, 256]),
            ("score", "float32", [1, 1]),
        ],
    },
}
# Deliberately finite CPU builtins only. New lowering operators require review.
# Numeric values: TensorFlow v2.20.0 compiler/mlir/lite/schema/schema.fbs.
BUILTINS = {
    0: "ADD", 2: "CONCATENATION", 3: "CONV_2D", 4: "DEPTHWISE_CONV_2D",
    6: "DEQUANTIZE", 9: "FULLY_CONNECTED", 14: "LOGISTIC", 17: "MAX_POOL_2D",
    18: "MUL", 19: "RELU", 21: "RELU6", 22: "RESHAPE", 23: "RESIZE_BILINEAR",
    25: "SOFTMAX", 28: "TANH", 34: "PAD", 36: "GATHER", 39: "TRANSPOSE",
    40: "MEAN", 41: "SUB", 42: "DIV", 43: "SQUEEZE", 45: "STRIDED_SLICE",
    47: "EXP", 49: "SPLIT", 53: "CAST", 55: "MAXIMUM", 57: "MINIMUM",
    58: "LESS", 59: "NEG", 61: "GREATER", 62: "GREATER_EQUAL", 63: "LESS_EQUAL",
    64: "SELECT", 65: "SLICE", 66: "SIN", 67: "TRANSPOSE_CONV", 69: "TILE",
    70: "EXPAND_DIMS", 71: "EQUAL", 72: "NOT_EQUAL", 74: "SUM", 75: "SQRT", 76: "RSQRT",
    77: "SHAPE", 78: "POW", 83: "PACK", 84: "LOGICAL_OR", 86: "LOGICAL_AND",
    87: "LOGICAL_NOT", 88: "UNPACK", 94: "FILL", 96: "RANGE", 99: "SQUARED_DIFFERENCE",
    102: "SPLIT_V", 108: "COS", 114: "QUANTIZE", 123: "SELECT_V2",
    126: "BATCH_MATMUL", 130: "BROADCAST_TO", 150: "GELU",
}


class VerificationError(ValueError):
    """Untrusted or unsupported bytes; no admission was granted."""


def require(condition, message):
    if not condition:
        raise VerificationError(message)


def keys(value, expected):
    require(type(value) is dict and set(value) == set(expected), "unexpected object fields")


def _json_value(value, depth=0):
    require(depth <= 32, "JSON nesting limit")
    if type(value) is dict:
        require(len(value) <= 512, "JSON object limit")
        for key, item in value.items():
            require(type(key) is str and key.isascii() and len(key) <= 256, "JSON key")
            _json_value(item, depth + 1)
    elif type(value) is list:
        require(len(value) <= MAX_OPERATORS, "JSON array limit")
        for item in value:
            _json_value(item, depth + 1)
    elif type(value) is str:
        require(value.isascii() and len(value) <= 4096, "JSON string")
    else:
        require(type(value) is bool or (type(value) is int and -(2**63) <= value < 2**63), "JSON scalar")


def canonical_bytes(value):
    _json_value(value)
    data = (json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":"), allow_nan=False) + "\n").encode("ascii")
    require(len(data) <= MAX_JSON, "JSON byte limit")
    return data


def parse_canonical(data):
    require(len(data) <= MAX_JSON, "JSON byte limit")
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON field")
            result[key] = value
        return result
    try:
        value = json.loads(data.decode("ascii"), object_pairs_hook=pairs)
        require(data == canonical_bytes(value), "noncanonical JSON bytes")
        return value
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        raise VerificationError("invalid JSON") from error


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def checked_relative(name):
    require(type(name) is str and name.isascii() and 0 < len(name) <= 240, "artifact path")
    path = PurePosixPath(name)
    require(not path.is_absolute() and "\\" not in name and ":" not in name, "artifact path")
    require(all(re.fullmatch(r"[A-Za-z0-9_.-]+", part) and part not in (".", "..") for part in path.parts), "artifact path")
    require(str(path) == name, "noncanonical artifact path")
    return path.parts


@contextlib.contextmanager
def checked_open(root, name):
    """Walk held directory descriptors without following symlinks or races."""
    parts = checked_relative(name)
    root = Path(root).absolute()
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW
    held = []
    try:
        current = os.open(root.anchor, flags | os.O_DIRECTORY)
        held.append(current)
        for part in root.parts[1:] + parts[:-1]:
            current = os.open(part, flags | os.O_DIRECTORY, dir_fd=current)
            held.append(current)
        fd = os.open(parts[-1], flags | os.O_NONBLOCK, dir_fd=current)
        held.append(fd)
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1, "artifact must be a private regular file")
        with os.fdopen(os.dup(fd), "rb") as stream:
            yield stream, before
        after = os.fstat(fd)
        require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns), "artifact changed while reading")
    except OSError as error:
        raise VerificationError("artifact unavailable or unsafe path") from error
    finally:
        for fd in reversed(held):
            os.close(fd)


def read_checked(root, name, limit=MAX_JSON):
    with checked_open(root, name) as (stream, info):
        require(0 < info.st_size <= limit, "artifact byte limit")
        data = stream.read(limit + 1)
        require(len(data) == info.st_size, "artifact size changed")
        return data


def verify_child(root, child, limit=MAX_ARTIFACT):
    require(type(child["bytes"]) is int and 0 < child["bytes"] <= limit, "artifact size")
    require(type(child["sha256"]) is str and re.fullmatch(r"[0-9a-f]{64}", child["sha256"]), "artifact hash")
    with checked_open(root, child["path"]) as (stream, info):
        require(info.st_size == child["bytes"], "artifact size mismatch")
        digest = hashlib.sha256()
        remaining = info.st_size
        while remaining:
            block = stream.read(min(65536, remaining))
            require(bool(block), "short artifact")
            digest.update(block)
            remaining -= len(block)
        require(digest.hexdigest() == child["sha256"], "artifact hash mismatch")


def parse_catalog(data):
    value = parse_canonical(data)
    require(value.get("schema") == "vw-assisted-candidate-catalog-v1" and value.get("production_admissions") == [], "candidate is not production admission")
    return value


def load_catalog():
    path = Path(__file__).absolute().parent
    return parse_catalog(read_checked(path, "model-runtime-candidate.json"))


class Flatbuffer:
    """Only bounded structural reads, with no interpreter or native parser."""
    def __init__(self, data):
        require(8 <= len(data) <= MAX_ARTIFACT and data[4:8] == b"TFL3", "TFLite identifier")
        self.data = data
        self.reads = 0

    def charge(self, amount):
        self.reads += amount
        require(self.reads <= MAX_METADATA_READS, "flatbuffer metadata work limit")

    def unpack(self, fmt, offset):
        self.charge(1)
        size = struct.calcsize(fmt)
        require(type(offset) is int and 0 <= offset <= len(self.data) - size, "flatbuffer bounds")
        return struct.unpack_from(fmt, self.data, offset)[0]

    def target(self, offset):
        delta = self.unpack("<I", offset)
        require(delta >= 4, "flatbuffer relative offset")
        target = offset + delta
        self.unpack("<B", target)
        return target

    def field(self, table, number):
        vtable = table - self.unpack("<i", table)
        length = self.unpack("<H", vtable)
        object_size = self.unpack("<H", vtable + 2)
        require(length >= 4 and length % 2 == 0 and object_size >= 4, "flatbuffer table")
        self.unpack("<B", vtable + length - 1)
        self.unpack("<B", table + object_size - 1)
        if 4 + 2 * number >= length:
            return 0
        offset = self.unpack("<H", vtable + 4 + 2 * number)
        require(offset == 0 or 4 <= offset < object_size, "flatbuffer field")
        return table + offset if offset else 0

    def scalar(self, table, field, fmt="<I", default=0):
        address = self.field(table, field)
        return self.unpack(fmt, address) if address else default

    def vector(self, table, field, size=4, cap=MAX_TENSORS):
        address = self.field(table, field)
        if not address:
            return []
        start = self.target(address)
        count = self.unpack("<I", start)
        require(count <= cap, "flatbuffer vector limit")
        self.charge(count)
        if count:
            self.unpack("<B", start + 4 + count * size - 1)
        return [start + 4 + index * size for index in range(count)]

    def tables(self, table, field, cap=MAX_TENSORS):
        return [self.target(at) for at in self.vector(table, field, cap=cap)]

    def integers(self, table, field, cap=MAX_TENSORS):
        return [self.unpack("<i", at) for at in self.vector(table, field, cap=cap)]

    def string(self, table, field):
        address = self.field(table, field)
        if not address:
            return ""
        start = self.target(address)
        count = self.unpack("<I", start)
        require(count <= 4096 and self.unpack("<B", start + 4 + count) == 0, "flatbuffer string")
        try:
            return self.data[start + 4:start + 4 + count].decode("ascii")
        except UnicodeError as error:
            raise VerificationError("non-ASCII tensor/operator name") from error


def inspect_tflite(data, role):
    require(role in CONTRACTS, "graph role")
    fb = Flatbuffer(data)
    model = fb.target(0)
    require(fb.scalar(model, 0) == 3, "TFLite schema version")
    graphs = fb.tables(model, 2, cap=2)
    require(len(graphs) == 1, "exactly one static subgraph required")
    graph = graphs[0]
    tensors = fb.tables(graph, 0)
    require(bool(tensors), "missing tensors")
    codes = []
    for opcode in fb.tables(model, 1, cap=512):
        code = max(fb.scalar(opcode, 0, "<b"), fb.scalar(opcode, 3, "<i"))
        version = fb.scalar(opcode, 2, "<i", 1)
        require(code in BUILTINS and 1 <= version <= 32 and not fb.string(opcode, 1), "unsupported/Flex/custom/vendor operator")
        codes.append({"code": code, "name": BUILTINS[code], "version": version})
    require(bool(codes), "missing operators")
    require(len({(row["code"], row["version"]) for row in codes}) == len(codes), "duplicate opcode entry")
    operators = fb.tables(graph, 3, cap=MAX_OPERATORS)
    require(bool(operators), "empty graph")
    used = set()
    for operator in operators:
        index = fb.scalar(operator, 0)
        require(index < len(codes), "operator index")
        used.add(index)
        for field in (1, 2, 8):
            lower = -1 if field == 1 else 0
            require(all(lower <= index < len(tensors) for index in fb.integers(operator, field)), "operator tensor index")
        require(not fb.vector(operator, 5, size=1), "custom operator options")
        require(not any(fb.unpack("<?", at) for at in fb.vector(operator, 7, size=1)), "mutable operator input")
        require(fb.scalar(operator, 9, "<Q") == 0 and fb.scalar(operator, 10, "<Q") == 0, "external custom options")
    require(len(used) == len(codes), "unused opcode entry")
    buffers = fb.tables(model, 4)
    require(bool(buffers), "missing tensor buffers")
    for buffer in buffers:
        require(fb.scalar(buffer, 1, "<Q") == 0 and fb.scalar(buffer, 2, "<Q") == 0, "external tensor buffer")
        address = fb.field(buffer, 0)
        if address:
            start = fb.target(address)
            count = fb.unpack("<I", start)
            require(count <= MAX_ARTIFACT, "tensor buffer byte limit")
            if count:
                fb.unpack("<B", start + 4 + count - 1)
    for tensor in tensors:
        require(fb.scalar(tensor, 2) < len(buffers), "tensor buffer index")
        require(not fb.scalar(tensor, 5, "<?"), "variable tensor")
        shape = fb.integers(tensor, 0, cap=8)
        signature = fb.integers(tensor, 7, cap=8)
        require(all(0 <= n <= 16_777_216 for n in shape) and (not signature or signature == shape), "dynamic tensor")
        require(fb.scalar(tensor, 1, "<b") not in (13, 14), "resource/variant tensor")
    signatures = fb.tables(model, 7, cap=2)
    require(len(signatures) == 1, "one named signature required")
    signature = signatures[0]
    require(fb.string(signature, 2) == "serving_default" and fb.scalar(signature, 4) == 0, "graph signature")
    report = {"signature_key": "serving_default"}
    for direction, field in (("inputs", 0), ("outputs", 1)):
        bindings = {}
        for binding in fb.tables(signature, field, cap=8):
            name = fb.string(binding, 0)
            require(name not in bindings, "duplicate signature name")
            bindings[name] = fb.scalar(binding, 1)
        expected = CONTRACTS[role][direction]
        require(set(bindings) == {name for name, _, _ in expected}, "signature binding names")
        require(len(set(bindings.values())) == len(bindings), "duplicate signature tensor index")
        indices = fb.integers(graph, 1 if direction == "inputs" else 2, cap=8)
        require(len(set(indices)) == len(indices), "duplicate graph IO tensor index")
        require(len(indices) == len(expected) and set(indices) == set(bindings.values()), "graph IO binding")
        rows = []
        for name, dtype, shape in expected:
            index = bindings[name]
            require(0 <= index < len(tensors), "IO tensor index")
            tensor = tensors[index]
            require(fb.scalar(tensor, 1, "<b") == {"float32": 0, "int32": 2}[dtype], "IO dtype")
            require(fb.integers(tensor, 0, cap=8) == shape, "IO shape")
            rows.append({"name": name, "signature_name": name, "tensor_name": fb.string(tensor, 3), "tensor_index": index, "dtype": dtype, "shape": shape})
        report[direction] = rows
    report["operators"] = sorted((codes[index] for index in used), key=lambda row: (row["code"], row["version"]))
    return report


def verify_model(root, manifest, catalog=None):
    catalog = catalog or load_catalog()
    keys(manifest, ("schema", "status", "model", "source_commit", "checkpoint", "graph_contract", "artifacts", "preprocessing", "postprocessing", "converter"))
    require(manifest["schema"] == "vw-assisted-model-v1" and manifest["status"] == "pending-admission", "model manifest status")
    require(manifest["model"] == "MobileSAM-vit_t" and manifest["source_commit"] == catalog["model"]["source_commit"], "model source")
    checkpoint = {key: catalog["model"]["checkpoint"][key] for key in ("bytes", "sha256")}
    require(manifest["checkpoint"] == checkpoint and manifest["graph_contract"] == GRAPH_CONTRACT, "model graph provenance")
    require(manifest["preprocessing"] == PREPROCESSING and manifest["postprocessing"] == POSTPROCESSING, "pixel policy")
    require(type(manifest["artifacts"]) is list and len(manifest["artifacts"]) == 2, "model artifact count")
    total = 0
    for artifact, role in zip(manifest["artifacts"], CONTRACTS):
        keys(artifact, ("role", "path", "bytes", "sha256", "signature_key", "inputs", "outputs", "operators"))
        require(artifact["role"] == role and artifact["path"] == {"encoder": "encoder.tflite", "prompt_decoder": "prompt_decoder.tflite"}[role], "model artifact role/path")
        verify_child(root, artifact)
        total += artifact["bytes"]
        require(total <= MAX_TOTAL, "model artifact total")
        data = read_checked(root, artifact["path"], MAX_ARTIFACT)
        require(len(data) == artifact["bytes"] and sha256(data) == artifact["sha256"], "artifact changed before inspection")
        inspected = inspect_tflite(data, role)
        supplied = {key: artifact[key] for key in inspected}
        require(canonical_bytes(supplied) == canonical_bytes(inspected), "artifact tensor/operator manifest mismatch")
    converter = manifest["converter"]
    keys(converter, ("packages", "files"))
    require(converter["packages"] == catalog["converter"]["packages"], "converter version pins")
    require(type(converter["files"]) is list and len(converter["files"]) == 3, "converter provenance")
    for row, name in zip(converter["files"], ("convert_mobile_sam.py", "verify_artifacts.py", "model-runtime-candidate.json")):
        keys(row, ("path", "bytes", "sha256"))
        require(row["path"] == name, "converter provenance path")
        verify_child(Path(__file__).absolute().parent, row, MAX_JSON)
    return sha256(canonical_bytes(manifest))


def runtime_manifest(root, catalog=None):
    catalog = catalog or load_catalog()
    artifacts = []
    for row in catalog["runtime"]["artifacts"]:
        child = {key: row[key] for key in ("role", "path", "bytes", "sha256")}
        verify_child(root, child)
        artifacts.append(child)
    return {"schema": "vw-assisted-runtime-v1", "status": "pending-admission", "runtime": "LiteRT", "version": "2.3.0", "artifacts": artifacts}


def verify_runtime(root, manifest, catalog=None):
    require(canonical_bytes(manifest) == canonical_bytes(runtime_manifest(root, catalog)), "runtime manifest mismatch")
    return sha256(canonical_bytes(manifest))


def pack_prompts(points, box=None):
    """Reference layout only: coordinates are already resized SAM pixel edges."""
    require(type(points) in (list, tuple) and len(points) <= 64, "point count")
    rows = []
    for point in points:
        require(len(point) == 3 and type(point[2]) is int and point[2] in (0, 1), "point label")
        x, y, label = point
        require(all(type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1024 for v in (x, y)), "point coordinate")
        rows.append(([float(x), float(y)], int(label)))
    if box is not None:
        require(len(box) == 4 and all(type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1024 for v in box), "box coordinates")
        x0, y0, x1, y1 = box
        require(x0 < x1 and y0 < y1, "empty box")
        rows.extend((([float(x0), float(y0)], 2), ([float(x1), float(y1)], 3)))
    else:
        require(bool(rows), "point or box required")
        rows.append(([0.0, 0.0], -1))
    count = len(rows)
    rows.extend([([0.0, 0.0], -1)] * (66 - count))
    return {"point_coords": [[xy for xy, _ in rows]], "point_labels": [[label for _, label in rows]], "point_valid": [[int(index < count) for index in range(66)]], "token_count": [count]}


def write_new(path, data):
    path = Path(path).absolute()
    checked_relative(path.name)
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    held = []
    try:
        current = os.open(path.anchor, flags)
        held.append(current)
        for part in path.parts[1:-1]:
            current = os.open(part, flags, dir_fd=current)
            held.append(current)
        fd = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW, 0o600, dir_fd=current)
        with os.fdopen(fd, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
    except OSError as error:
        raise VerificationError("output already exists or has unsafe parent") from error
    finally:
        for fd in reversed(held):
            os.close(fd)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("model", "runtime", "runtime-manifest", "catalog"))
    parser.add_argument("--root", type=Path)
    parser.add_argument("--manifest")
    parser.add_argument("--sha256")
    args = parser.parse_args(argv)
    if args.kind == "catalog":
        catalog = load_catalog()
        print(json.dumps({"status": catalog["status"], "production_admissions": 0}, sort_keys=True))
        return 0
    require(args.root is not None and args.manifest is not None, "root and manifest required")
    checked_relative(args.manifest)
    if args.kind == "runtime-manifest":
        manifest = runtime_manifest(args.root)
        data = canonical_bytes(manifest)
        write_new(args.root / args.manifest, data)
        print(json.dumps({"sha256": sha256(data), "status": "pending-admission"}, sort_keys=True))
        return 0
    data = read_checked(args.root, args.manifest)
    manifest = parse_canonical(data)
    digest = (verify_model if args.kind == "model" else verify_runtime)(args.root, manifest)
    require(args.sha256 is None or args.sha256 == digest, "manifest identity mismatch")
    print(json.dumps({"sha256": digest, "status": "pending-admission", "production_admitted": False}, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (VerificationError, KeyError, TypeError, OverflowError) as error:
        print(f"verification refused: {error}", file=sys.stderr)
        raise SystemExit(2) from None
