"""Synthetic metadata/layout checks only: no torch, model, or inference imports."""
from __future__ import annotations

import ast
import copy
import contextlib
import hashlib
import io
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

TOOLS = Path(__file__).absolute().parents[1]
sys.path.insert(0, str(TOOLS))
import verify_artifacts as verify
import convert_mobile_sam as convert


class MetadataBuilder:
    """Minimal synthetic FlatBuffer metadata, deliberately not an ML model."""
    def __init__(self):
        self.data = bytearray(b"\0\0\0\0TFL3")
        self.vtables = {}

    def align(self):
        self.data.extend(b"\0" * (-len(self.data) % 4))

    def table(self, fields):
        self.align()
        vtable = len(self.data)
        self.data.extend(struct.pack("<HH", 4 + 2 * fields, 4 + 4 * fields))
        self.data.extend(b"\0" * (2 * fields))
        self.align()
        table = len(self.data)
        self.data.extend(struct.pack("<i", table - vtable) + b"\0" * (4 * fields))
        self.vtables[table] = vtable
        return table

    def scalar(self, table, field, value):
        struct.pack_into("<H", self.data, self.vtables[table] + 4 + 2 * field, 4 + 4 * field)
        struct.pack_into("<i", self.data, table + 4 + 4 * field, value)

    def ref(self, table, field, target):
        struct.pack_into("<H", self.data, self.vtables[table] + 4 + 2 * field, 4 + 4 * field)
        position = table + 4 + 4 * field
        struct.pack_into("<I", self.data, position, target - position)

    def vector(self, values):
        self.align()
        position = len(self.data)
        self.data.extend(struct.pack("<I", len(values)))
        self.data.extend(b"".join(struct.pack("<i", value) for value in values))
        return position

    def references(self, count):
        return self.vector([0] * count)

    def set_reference(self, vector, index, target):
        position = vector + 4 + 4 * index
        struct.pack_into("<I", self.data, position, target - position)

    def string(self, text):
        self.align()
        position = len(self.data)
        raw = text.encode("ascii")
        self.data.extend(struct.pack("<I", len(raw)) + raw + b"\0")
        return position


def synthetic_metadata(role="encoder", code=0, custom="", dtype_override=None, dynamic=False,
                       alias_graph_inputs=False, alias_signature_inputs=False):
    b = MetadataBuilder()
    model = b.table(8)
    struct.pack_into("<I", b.data, 0, model)
    b.scalar(model, 0, 3)
    opcode_vector = b.references(1)
    b.ref(model, 1, opcode_vector)
    opcode = b.table(4)
    b.set_reference(opcode_vector, 0, opcode)
    b.scalar(opcode, 0, min(code, 127))
    b.scalar(opcode, 2, 1)
    b.scalar(opcode, 3, code)
    if custom:
        b.ref(opcode, 1, b.string(custom))
    graph_vector = b.references(1)
    b.ref(model, 2, graph_vector)
    graph = b.table(5)
    b.set_reference(graph_vector, 0, graph)
    contract = verify.CONTRACTS[role]
    all_tensors = contract["inputs"] + contract["outputs"]
    tensor_vector = b.references(len(all_tensors))
    b.ref(graph, 0, tensor_vector)
    for index, (name, dtype, shape) in enumerate(all_tensors):
        tensor = b.table(8)
        b.set_reference(tensor_vector, index, tensor)
        b.ref(tensor, 0, b.vector(shape))
        b.scalar(tensor, 1, dtype_override if dtype_override is not None and index == 0 else {"float32": 0, "int32": 2}[dtype])
        b.ref(tensor, 3, b.string("raw_" + name))
        if dynamic and index == 0:
            b.ref(tensor, 7, b.vector([-1] + shape[1:]))
    inputs = list(range(len(contract["inputs"])))
    outputs = list(range(len(inputs), len(all_tensors)))
    if alias_graph_inputs:
        inputs[3] = inputs[2]  # point_valid aliases point_labels: same dtype/shape.
    b.ref(graph, 1, b.vector(list(reversed(inputs))))
    b.ref(graph, 2, b.vector(list(reversed(outputs))))
    operator_vector = b.references(1)
    b.ref(graph, 3, operator_vector)
    operator = b.table(3)
    b.set_reference(operator_vector, 0, operator)
    b.ref(operator, 1, b.vector(inputs))
    b.ref(operator, 2, b.vector(outputs))
    buffers = b.references(1)
    b.ref(model, 4, buffers)
    buffer = b.table(3)
    b.set_reference(buffers, 0, buffer)
    signatures = b.references(1)
    b.ref(model, 7, signatures)
    signature = b.table(5)
    b.set_reference(signatures, 0, signature)
    b.ref(signature, 2, b.string("serving_default"))
    for field, rows, offset in ((0, contract["inputs"], 0), (1, contract["outputs"], len(inputs))):
        maps = b.references(len(rows))
        b.ref(signature, field, maps)
        for index, (name, _, _) in enumerate(rows):
            entry = b.table(2)
            b.set_reference(maps, index, entry)
            bound_index = 2 if field == 0 and index == 3 and alias_signature_inputs else offset + index
            b.scalar(entry, 1, bound_index)
            b.ref(entry, 0, b.string(name))
    return bytes(b.data)


class CanonicalTests(unittest.TestCase):
    def test_identity_is_exact_sorted_ascii_compact_json_and_newline(self):
        raw = verify.canonical_bytes({"z": [1, True], "a": "value"})
        self.assertEqual(raw, b'{"a":"value","z":[1,true]}\n')
        self.assertEqual(verify.sha256(raw), hashlib.sha256(raw).hexdigest())
        self.assertEqual(verify.parse_canonical(raw), {"a": "value", "z": [1, True]})

    def test_noncanonical_duplicate_float_null_unicode_and_depth_refuse(self):
        for raw in (b'{"a":1,"a":1}\n', b'{"a": 1}\n', b'{"a":1}', b'{"a":1.0}\n', b'{"a":null}\n', b'{"a":"\\u00e9"}\n', b'{"a":NaN}\n'):
            with self.subTest(raw=raw), self.assertRaises(verify.VerificationError):
                verify.parse_canonical(raw)
        with self.assertRaises(verify.VerificationError):
            verify.parse_canonical(b"[" * 100 + b"0" + b"]" * 100 + b"\n")

    def test_candidate_has_no_production_or_converted_artifact_admission(self):
        candidate = verify.load_catalog()
        self.assertEqual(candidate["production_admissions"], [])
        self.assertEqual(candidate["model"]["converted_artifacts"], [])
        self.assertEqual(candidate["runtime"]["version"], "2.3.0")


class FileTests(unittest.TestCase):
    def test_hash_size_capacity_and_no_clobber(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            verify.write_new(root / "child.bin", b"test")
            child = {"path": "child.bin", "bytes": 4, "sha256": verify.sha256(b"test")}
            verify.verify_child(root, child)
            for changed in ({**child, "bytes": 5}, {**child, "sha256": "0" * 64}, {**child, "bytes": True}):
                with self.assertRaises(verify.VerificationError):
                    verify.verify_child(root, changed)
            with self.assertRaises(verify.VerificationError):
                verify.write_new(root / "child.bin", b"replace")
            self.assertEqual((root / "child.bin").read_bytes(), b"test")

    def test_traversal_symlinks_hardlinks_and_nonregular_files_refuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "child.bin").write_bytes(b"test")
            (root / "link").symlink_to(root / "child.bin")
            (root / "dir-link").symlink_to(root, target_is_directory=True)
            os.mkfifo(root / "pipe")
            for name in ("../child.bin", "./child.bin", "/child.bin", "a//b", "a\\b", "link", "dir-link/child.bin", "pipe"):
                with self.subTest(name=name), self.assertRaises(verify.VerificationError):
                    verify.read_checked(root, name)
            os.link(root / "child.bin", root / "hardlink")
            with self.assertRaises(verify.VerificationError):
                verify.read_checked(root, "hardlink")
            with self.assertRaises(verify.VerificationError):
                verify.write_new(root / "dir-link" / "new.bin", b"data")

    def test_empty_oversize_and_changed_held_file_refuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "empty").write_bytes(b"")
            (root / "file").write_bytes(b"abcd")
            for name, limit in (("empty", 4), ("file", 3)):
                with self.assertRaises(verify.VerificationError):
                    verify.read_checked(root, name, limit)
            with self.assertRaises(verify.VerificationError):
                with verify.checked_open(root, "file") as (stream, _):
                    self.assertEqual(stream.read(), b"abcd")
                    (root / "file").write_bytes(b"changed")

    @unittest.skipUnless(sys.platform == "linux", "Linux atomic no-replace contract")
    def test_atomic_directory_publish_never_replaces_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "stage").mkdir()
            (root / "stage" / "artifact").write_bytes(b"owned")
            (root / "output").mkdir()
            with convert.directory_fd(root) as parent:
                with self.assertRaises(verify.VerificationError):
                    convert.atomic_publish(parent, "stage", "output")
                self.assertTrue((root / "stage" / "artifact").exists())
                (root / "output").rmdir()
                convert.atomic_publish(parent, "stage", "output")
            self.assertFalse((root / "stage").exists())
            self.assertEqual((root / "output" / "artifact").read_bytes(), b"owned")


class FlatbufferTests(unittest.TestCase):
    def test_named_signatures_bind_exact_contracts_without_assuming_io_order(self):
        for role in verify.CONTRACTS:
            report = verify.inspect_tflite(synthetic_metadata(role), role)
            self.assertEqual(report["signature_key"], "serving_default")
            for direction in ("inputs", "outputs"):
                actual = [(row["name"], row["dtype"], row["shape"]) for row in report[direction]]
                self.assertEqual(actual, verify.CONTRACTS[role][direction])
                self.assertTrue(all(row["signature_name"] == row["name"] and row["tensor_name"].startswith("raw_") for row in report[direction]))

    def test_flex_custom_vendor_delegate_unknown_dtype_and_dynamic_refuse(self):
        for kwargs in ({"code": 32, "custom": "FlexEinsum"}, {"code": 32, "custom": "vendor"}, {"code": 51}, {"code": 999}, {"dtype_override": 1}, {"dynamic": True}):
            with self.subTest(kwargs=kwargs), self.assertRaises(verify.VerificationError):
                verify.inspect_tflite(synthetic_metadata(**kwargs), "encoder")

    def test_truncated_and_unbounded_offsets_vectors_refuse(self):
        raw = synthetic_metadata()
        for cut in (0, 7, 20, len(raw) - 1):
            with self.subTest(cut=cut), self.assertRaises(verify.VerificationError):
                verify.inspect_tflite(raw[:cut], "encoder")
        bad = bytearray(raw)
        struct.pack_into("<I", bad, 0, 2**32 - 1)
        with self.assertRaises(verify.VerificationError):
            verify.inspect_tflite(bytes(bad), "encoder")
        bad = bytearray(raw)
        fb = verify.Flatbuffer(bad)
        model = fb.target(0)
        vector = fb.target(fb.field(model, 2))
        struct.pack_into("<I", bad, vector, 2**32 - 1)
        with self.assertRaises(verify.VerificationError):
            verify.inspect_tflite(bytes(bad), "encoder")

    def test_swapped_role_contract_refuses(self):
        with self.assertRaises(verify.VerificationError):
            verify.inspect_tflite(synthetic_metadata("encoder"), "prompt_decoder")

    def test_signature_tensor_indices_must_be_unique_even_when_graph_aliases_match(self):
        # Both aliases previously passed the set-equality check: labels and
        # valid masks have identical dtype/shape but independent meanings.
        for alias_graph in (False, True):
            with self.subTest(alias_graph=alias_graph), self.assertRaisesRegex(verify.VerificationError, "duplicate signature tensor index"):
                verify.inspect_tflite(synthetic_metadata("prompt_decoder", alias_graph_inputs=alias_graph,
                    alias_signature_inputs=True), "prompt_decoder")

    def test_graph_io_tensor_indices_must_be_unique(self):
        with self.assertRaisesRegex(verify.VerificationError, "duplicate graph IO tensor index"):
            verify.inspect_tflite(synthetic_metadata("prompt_decoder", alias_graph_inputs=True), "prompt_decoder")

    def test_metadata_work_budget_stops_aliased_repeated_reads(self):
        fb = verify.Flatbuffer(synthetic_metadata())
        fb.reads = verify.MAX_METADATA_READS
        with self.assertRaises(verify.VerificationError):
            fb.unpack("<I", 0)


class PromptTests(unittest.TestCase):
    def test_no_box_has_one_valid_sentinel_and_padding_is_inactive(self):
        for count in (1, 2, 63, 64):
            packed = verify.pack_prompts([(4, 5, i % 2) for i in range(count)])
            self.assertEqual(packed["token_count"], [count + 1])
            valid = packed["point_valid"][0]
            labels = packed["point_labels"][0]
            self.assertEqual(sum(valid), count + 1)
            self.assertEqual(sum(label == -1 for label, active in zip(labels, valid) if active), 1)
            self.assertEqual(labels[count], -1)
            self.assertTrue(all(active == 0 for active in valid[count + 1:]))

    def test_box_only_and_64_points_box_use_two_valid_corners_no_sentinel(self):
        for count in (0, 1, 64):
            packed = verify.pack_prompts([(4, 5, 1)] * count, (0, 0, 100, 100))
            self.assertEqual(packed["token_count"], [count + 2])
            active = packed["point_labels"][0][:count + 2]
            self.assertEqual(active[-2:], [2, 3])
            self.assertNotIn(-1, active)
            self.assertEqual(len(packed["point_coords"][0]), 66)

    def test_invalid_overflow_empty_nonfinite_and_box_refuse(self):
        for points, box in (([], None), ([(1, 1, 1)] * 65, None), ([(float("nan"), 1, 1)], None), ([(1, 1, 2)], None), ([], (0, 0, 0, 3)), ([], (0, 0, 1025, 3))):
            with self.subTest(points=points[:1], box=box), self.assertRaises(verify.VerificationError):
                verify.pack_prompts(points, box)


class ManifestTests(unittest.TestCase):
    def bundle(self, root):
        catalog = verify.load_catalog()
        artifacts = []
        for role in verify.CONTRACTS:
            data = synthetic_metadata(role)
            name = role + ".tflite"
            (root / name).write_bytes(data)
            artifacts.append({"role": role, "path": name, "bytes": len(data), "sha256": verify.sha256(data), **verify.inspect_tflite(data, role)})
        provenance = []
        for name in ("convert_mobile_sam.py", "verify_artifacts.py", "model-runtime-candidate.json"):
            data = verify.read_checked(TOOLS, name)
            provenance.append({"path": name, "bytes": len(data), "sha256": verify.sha256(data)})
        return {"schema": "vw-assisted-model-v1", "status": "pending-admission", "model": "MobileSAM-vit_t", "source_commit": catalog["model"]["source_commit"], "checkpoint": {key: catalog["model"]["checkpoint"][key] for key in ("bytes", "sha256")}, "graph_contract": verify.GRAPH_CONTRACT, "artifacts": artifacts, "preprocessing": verify.PREPROCESSING, "postprocessing": verify.POSTPROCESSING, "converter": {"packages": catalog["converter"]["packages"], "files": provenance}}

    def test_model_manifest_binds_both_children_provenance_and_canonical_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = self.bundle(root)
            digest = verify.verify_model(root, manifest)
            self.assertEqual(digest, hashlib.sha256(verify.canonical_bytes(manifest)).hexdigest())
            (root / "model-manifest.json").write_bytes(verify.canonical_bytes(manifest))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(verify.main(["model", "--root", str(root), "--manifest", "model-manifest.json", "--sha256", digest]), 0)
            with self.assertRaises(verify.VerificationError):
                verify.main(["model", "--root", str(root), "--manifest", "model-manifest.json", "--sha256", "0" * 64])
            (root / "prompt_decoder.tflite").write_bytes(b"changed")
            with self.assertRaises(verify.VerificationError):
                verify.verify_model(root, manifest)

    def test_claimed_admission_tensor_bool_and_provenance_tampering_refuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = self.bundle(root)
            for change in range(8):
                bad = copy.deepcopy(manifest)
                if change == 0:
                    bad["status"] = "admitted"
                elif change == 1:
                    bad["approval"] = True
                elif change == 2:
                    bad["artifacts"][0]["inputs"][0]["shape"][0] = True
                elif change == 3:
                    bad["artifacts"][0]["inputs"][0]["signature_name"] = "wrong"
                elif change == 4:
                    bad["converter"]["packages"]["torch"] = "0.0.0"
                elif change == 5:
                    bad["converter"]["files"][0]["sha256"] = "0" * 64
                elif change == 6:
                    bad["artifacts"][0]["operators"][0]["version"] = 2
                else:
                    bad["artifacts"][0]["path"] = "../encoder.tflite"
                with self.subTest(change=change), self.assertRaises(verify.VerificationError):
                    verify.verify_model(root, bad)

    def test_runtime_identity_binds_every_exact_local_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            children = []
            for index, role in enumerate(("litert-api", "litert", "litert-gpu")):
                data = bytes([index])  # Synthetic bytes, never presented as AARs.
                name = role + ".aar"
                (root / name).write_bytes(data)
                children.append({"role": role, "path": name, "bytes": len(data), "sha256": verify.sha256(data)})
            catalog = {"runtime": {"artifacts": children}}
            manifest = verify.runtime_manifest(root, catalog)
            self.assertEqual(verify.verify_runtime(root, manifest, catalog), verify.sha256(verify.canonical_bytes(manifest)))
            bad = copy.deepcopy(manifest)
            bad["artifacts"][0]["bytes"] = True
            with self.assertRaises(verify.VerificationError):
                verify.verify_runtime(root, bad, catalog)
            (root / children[-1]["path"]).write_bytes(b"x")
            with self.assertRaises(verify.VerificationError):
                verify.verify_runtime(root, manifest, catalog)


class ConverterBoundaryTests(unittest.TestCase):
    def test_import_and_help_never_import_torch_or_litert(self):
        self.assertNotIn("torch", sys.modules)
        self.assertNotIn("litert_torch", sys.modules)
        result = subprocess.run([sys.executable, str(TOOLS / "convert_mobile_sam.py"), "--help"], capture_output=True, text=True, timeout=10, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--source-root", result.stdout)

    def test_source_pins_are_checked_before_any_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "allowed.py").write_bytes(b"raise RuntimeError('must not execute')\n")
            catalog = {"model": {"source_files": [{"path": "allowed.py", "bytes": 35, "sha256": "0" * 64}]}}
            with self.assertRaises(verify.VerificationError):
                convert.verify_source(root, catalog)

    def test_bad_checkpoint_and_existing_output_refuse_before_package_checks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkpoint = root / "checkpoint.pt"
            checkpoint.write_bytes(b"evil")
            catalog = {"model": {"checkpoint": {"bytes": 4, "sha256": verify.sha256(b"good")}}}
            with mock.patch.object(convert, "conversion_catalog", return_value=catalog), \
                 mock.patch.object(convert, "verify_source", return_value={}), \
                 mock.patch.object(convert, "verify_versions", side_effect=AssertionError("must not reach package checks")):
                with self.assertRaises(verify.VerificationError):
                    convert.convert(root, checkpoint, root / "new-output")
                self.assertFalse((root / "new-output").exists())
                checkpoint.write_bytes(b"good")
                (root / "existing-output").mkdir()
                with self.assertRaises(verify.VerificationError):
                    convert.convert(root, checkpoint, root / "existing-output")

    def test_weights_only_checkpoint_none_strict_load_and_both_attention_masks(self):
        tree = ast.parse((TOOLS / "convert_mobile_sam.py").read_text())
        calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)]
        loads = [node for node in calls if isinstance(node.func, ast.Attribute) and node.func.attr == "load"]
        self.assertEqual(len(loads), 1)
        self.assertTrue(any(keyword.arg == "weights_only" and isinstance(keyword.value, ast.Constant) and keyword.value.value is True for keyword in loads[0].keywords))
        builders = [node for node in calls if isinstance(node.func, ast.Attribute) and node.func.attr == "build_sam_vit_t"]
        self.assertEqual(len(builders), 1)
        self.assertTrue(any(keyword.arg == "checkpoint" and isinstance(keyword.value, ast.Constant) and keyword.value.value is None for keyword in builders[0].keywords))
        masked = [node for node in calls if isinstance(node.func, ast.Attribute) and node.func.attr == "_attention" and len(node.args) == 5]
        self.assertEqual(len(masked), 3)  # first/subsequent self-attn and image->token
        self.assertTrue(all(isinstance(node.args[-1], ast.Name) and node.args[-1].id == "valid" for node in masked))

    def test_offline_guard_refuses_socket_without_making_network_call(self):
        with convert.offline_imports():
            with self.assertRaises(verify.VerificationError):
                convert.socket.create_connection(("invalid.example", 1))

    def test_generic_or_cuda_distribution_refuses_before_ml_import(self):
        catalog = verify.load_catalog()
        packages = catalog["converter"]["packages"]
        for generic in (True, False):
            def installed(name):
                return "2.12.0" if generic and name == "torch" else packages[name]
            with mock.patch.object(convert.sys, "version_info", SimpleNamespace(major=3, minor=11)), \
                 mock.patch.object(convert.platform, "machine", return_value="x86_64"), \
                 mock.patch.object(convert.importlib.metadata, "version", side_effect=installed), \
                 mock.patch.object(convert.importlib.metadata, "distribution", return_value=SimpleNamespace(requires=["nvidia-cudnn-cu13==9.20.0.48"])):
                with self.assertRaises(verify.VerificationError):
                    convert.verify_versions(catalog)
        self.assertNotIn("torch", sys.modules)

    def test_imported_cpu_version_and_build_flags_must_match(self):
        catalog = verify.load_catalog()
        fake = SimpleNamespace(__version__="2.12.0+cpu", version=SimpleNamespace(cuda=None, hip=None))
        convert.verify_imported_cpu(fake, catalog)
        fake.version.cuda = "13.0"
        with self.assertRaises(verify.VerificationError):
            convert.verify_imported_cpu(fake, catalog)
        fake.version.cuda = None
        fake.__version__ = "2.12.0"
        with self.assertRaises(verify.VerificationError):
            convert.verify_imported_cpu(fake, catalog)


class ToolSnapshotTests(unittest.TestCase):
    def fixture(self, root):
        for name in convert.TOOL_FILES:
            (root / name).write_bytes(verify.read_checked(TOOLS, name))
        return convert.capture_tool_snapshot(root)

    def test_provenance_retains_startup_bytes_and_detects_same_size_file_change(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = self.fixture(root)
            original = dict(snapshot)["convert_mobile_sam.py"]
            provenance = convert.tool_provenance(snapshot)
            changed = bytes([original[0] ^ 1]) + original[1:]
            (root / "convert_mobile_sam.py").write_bytes(changed)
            self.assertEqual(provenance[0]["sha256"], verify.sha256(original))
            self.assertNotEqual(provenance[0]["sha256"], verify.sha256(changed))
            self.assertIsInstance(snapshot, tuple)
            self.assertTrue(all(isinstance(data, bytes) for _, data in snapshot))
            with self.assertRaisesRegex(verify.VerificationError, "tool changed after startup"):
                convert.revalidate_tool_snapshot(root, snapshot)

    def test_configuration_uses_retained_snapshot_and_changed_disk_config_refuses(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = self.fixture(root)
            original = convert.conversion_catalog(snapshot)
            changed = copy.deepcopy(original)
            changed["model"]["checkpoint"]["sha256"] = "0" * 64
            (root / "model-runtime-candidate.json").write_bytes(verify.canonical_bytes(changed))
            self.assertEqual(convert.conversion_catalog(snapshot), original)
            with self.assertRaisesRegex(verify.VerificationError, "tool changed after startup"):
                convert.revalidate_tool_snapshot(root, snapshot)

    def test_changed_tool_prevents_publication_without_ml_import_or_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = self.fixture(root)
            (root / "verify_artifacts.py").write_bytes(b"changed verifier source\n")
            with convert.directory_fd(root) as parent, mock.patch.object(convert, "atomic_publish") as publish:
                with self.assertRaisesRegex(verify.VerificationError, "tool changed after startup"):
                    convert._publish_with_tool_snapshot(parent, "staging", "output", root, snapshot)
                publish.assert_not_called()
            self.assertNotIn("torch", sys.modules)
            self.assertNotIn("litert_torch", sys.modules)


if __name__ == "__main__":
    unittest.main()
