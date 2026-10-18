#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Derivative adapter of SAM's decoder/TwoWayTransformer architecture:
# Copyright (c) Meta Platforms, Inc. and affiliates.
# MobileSAM source commit and complete source license identity are pinned in
# model-runtime-candidate.json. TinyViT retains its Microsoft attribution.
"""Explicit offline MobileSAM conversion. Importing this file does no ML work."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import ctypes
import errno
import hashlib
import importlib
import importlib.abc
import importlib.metadata
import importlib.util
import os
from pathlib import Path
import platform
import socket
import sys
import uuid

import verify_artifacts as verify


TOOL_FILES = ("convert_mobile_sam.py", "verify_artifacts.py", "model-runtime-candidate.json")
TOOLS_ROOT = Path(__file__).absolute().parent


def capture_tool_snapshot(root):
    """Retain immutable source bytes before imports/conversion can run."""
    return tuple((name, verify.read_checked(root, name)) for name in TOOL_FILES)


def tool_provenance(snapshot):
    return [{"path": name, "bytes": len(data), "sha256": verify.sha256(data)} for name, data in snapshot]


def revalidate_tool_snapshot(root, snapshot):
    for name, original in snapshot:
        verify.require(verify.read_checked(root, name) == original, "conversion tool changed after startup: " + name)


STARTUP_TOOL_SNAPSHOT = capture_tool_snapshot(TOOLS_ROOT)


def conversion_catalog(snapshot):
    # Configuration and its manifest identity must describe the same retained
    # bytes, even if the on-disk candidate file changes during a long conversion.
    return verify.parse_catalog(dict(snapshot)["model-runtime-candidate.json"])


def verify_versions(catalog):
    verify.require(sys.platform == "linux", "converter requires Linux")
    verify.require(platform.machine() == "x86_64", "converter requires pinned x86_64 CPU wheels")
    version = f"{sys.version_info.major}.{sys.version_info.minor}"
    verify.require(version == catalog["converter"]["python"], "Python version pin")
    for name, expected in catalog["converter"]["packages"].items():
        try:
            actual = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError as error:
            raise verify.VerificationError("required converter package missing: " + name) from error
        verify.require(actual == expected, "installed version mismatch: " + name)
    dependencies = importlib.metadata.distribution("torch").requires or []
    verify.require(not any(row.strip().lower().replace("_", "-").startswith(("nvidia-", "cuda-")) for row in dependencies), "GPU/CUDA torch dependency metadata")
    # Refuse local modules that could shadow the version-checked distributions.
    for name, distribution in (("torch", "torch"), ("torchvision", "torchvision"), ("timm", "timm"), ("numpy", "numpy"), ("litert_torch", "litert-torch")):
        spec = importlib.util.find_spec(name)
        expected = importlib.metadata.distribution(distribution).locate_file(name + "/__init__.py").absolute()
        verify.require(spec is not None and Path(spec.origin or "").absolute() == expected, "shadowed converter package: " + name)
        verify.read_checked(expected.parent, expected.name, verify.MAX_JSON)


def verify_imported_cpu(torch, catalog):
    verify.require(str(torch.__version__) == catalog["converter"]["packages"]["torch"], "imported torch version mismatch")
    verify.require(torch.version.cuda is None and torch.version.hip is None, "CPU-only torch build required")


def verify_source(source_root, catalog):
    sources = {}
    for row in catalog["model"]["source_files"]:
        verify.verify_child(source_root, row, verify.MAX_JSON)
        data = verify.read_checked(source_root, row["path"], verify.MAX_JSON)
        verify.require(len(data) == row["bytes"] and verify.sha256(data) == row["sha256"], "source changed before import")
        sources[row["path"]] = data
    return sources


class PinnedSource(importlib.abc.MetaPathFinder, importlib.abc.Loader):
    """Execute only hash-checked in-memory source; never an unchecked __init__."""
    def __init__(self, sources, root):
        self.sources = sources
        self.root = Path(root).absolute()

    def find_spec(self, fullname, path=None, target=None):
        if not (fullname == "mobile_sam" or fullname.startswith("mobile_sam.")):
            return None
        relative = fullname.replace(".", "/")
        package = relative + "/__init__.py"
        module = relative + ".py"
        if fullname == "mobile_sam":
            # The upstream top-level convenience package imports predictor and
            # automatic-mask dependencies. A namespace needs none of that code.
            return importlib.util.spec_from_loader(fullname, self, is_package=True)
        verify.require(package in self.sources or module in self.sources, "unpinned MobileSAM import")
        return importlib.util.spec_from_loader(fullname, self, is_package=package in self.sources)

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        if module.__name__ == "mobile_sam":
            module.__path__ = []
            return
        relative = module.__name__.replace(".", "/")
        path = relative + ("/__init__.py" if module.__spec__.submodule_search_locations is not None else ".py")
        module.__file__ = str(self.root / path)
        exec(compile(self.sources[path], module.__file__, "exec"), module.__dict__)


@contextmanager
def offline_imports():
    """Refuse Python socket activity, including accidental dependency telemetry."""
    def denied(*args, **kwargs):
        raise verify.VerificationError("network access is forbidden during offline conversion")
    targets = [(socket.socket, "connect"), (socket.socket, "connect_ex"), (socket, "create_connection"), (socket, "getaddrinfo")]
    previous = [(owner, name, getattr(owner, name)) for owner, name in targets]
    for owner, name in targets:
        setattr(owner, name, denied)
    names = ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "HF_HUB_DISABLE_TELEMETRY", "DO_NOT_TRACK", "CUDA_VISIBLE_DEVICES")
    environment = {name: os.environ.get(name) for name in names}
    for name in names:
        os.environ[name] = "" if name == "CUDA_VISIBLE_DEVICES" else "1"
    try:
        yield
    finally:
        for owner, name, original in previous:
            setattr(owner, name, original)
        for name, original in environment.items():
            if original is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = original


def build_wrappers(torch, sam):
    """Reuse all upstream learned modules; only sparse-token masking is new.

    Invalid queries cannot affect valid queries because keys are masked in both
    token self-attention and image-to-token cross-attention on every block.
    Layer norms/MLPs act independently per token. Output tokens always remain
    valid. This is structural reasoning, not executed reference-parity evidence.
    """
    class Encoder(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.encoder = sam.image_encoder

        def forward(self, image):
            return {"image_embeddings": self.encoder(image)}

    class Decoder(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.prompt = sam.prompt_encoder
            self.decoder = sam.mask_decoder
            self.register_buffer("dense_pe", self.prompt.get_dense_pe())
            self.register_buffer("slot_indices", torch.arange(66, dtype=torch.int32).reshape(1, 66))

        def _attention(self, module, query, key, value, valid_keys=None):
            query = module._separate_heads(module.q_proj(query), module.num_heads)
            key = module._separate_heads(module.k_proj(key), module.num_heads)
            value = module._separate_heads(module.v_proj(value), module.num_heads)
            scores = torch.matmul(query, key.transpose(-2, -1)) / (query.shape[-1] ** 0.5)
            if valid_keys is not None:
                scores = torch.where(valid_keys[:, None, None, :], scores, torch.full_like(scores, float("-inf")))
            attended = torch.matmul(torch.softmax(scores, dim=-1), value)
            return module.out_proj(module._recombine_heads(attended))

        def _transformer(self, image, position, tokens, valid):
            transformer = self.decoder.transformer
            queries = tokens
            keys = image.flatten(2).permute(0, 2, 1)
            image_position = position.flatten(2).permute(0, 2, 1)
            for block in transformer.layers:
                if block.skip_first_layer_pe:
                    queries = self._attention(block.self_attn, queries, queries, queries, valid)
                else:
                    positioned = queries + tokens
                    queries = queries + self._attention(block.self_attn, positioned, positioned, queries, valid)
                queries = block.norm1(queries)
                queries = block.norm2(queries + self._attention(block.cross_attn_token_to_image, queries + tokens, keys + image_position, keys))
                queries = block.norm3(queries + block.mlp(queries))
                keys = block.norm4(keys + self._attention(block.cross_attn_image_to_token, keys + image_position, queries + tokens, queries, valid))
            queries = transformer.norm_final_attn(queries + self._attention(transformer.final_attn_token_to_image, queries + tokens, keys + image_position, keys))
            return queries, keys

        def forward(self, image_embeddings, point_coords, point_labels, point_valid, token_count):
            # Upstream adds 0.5 before random Fourier encoding for points AND
            # corners. A valid -1 sentinel has only not_a_point_embed, no PE.
            normalized = (point_coords + 0.5) / 1024.0
            phase = ((normalized * 2.0 - 1.0) @ self.prompt.pe_layer.positional_encoding_gaussian_matrix) * (2.0 * 3.141592653589793)
            positional = torch.cat((torch.sin(phase), torch.cos(phase)), dim=-1)
            sparse = torch.where((point_labels == -1).unsqueeze(-1), self.prompt.not_a_point_embed.weight.reshape(1, 1, 256), positional)
            for label in range(4):
                sparse = sparse + (point_labels == label).unsqueeze(-1).to(torch.float32) * self.prompt.point_embeddings[label].weight.reshape(1, 1, 256)
            prefix = torch.cat((self.decoder.iou_token.weight, self.decoder.mask_tokens.weight), dim=0).unsqueeze(0)
            tokens = torch.cat((prefix, sparse), dim=1)
            active = (point_valid == 1) & (self.slot_indices < token_count.reshape(1, 1))
            valid = torch.cat((torch.ones((1, 5), dtype=torch.bool, device=active.device), active), dim=1)
            dense = self.prompt.no_mask_embed.weight.reshape(1, 256, 1, 1)
            states, image = self._transformer(image_embeddings + dense, self.dense_pe, tokens, valid)
            upscaled = self.decoder.output_upscaling(image.transpose(1, 2).reshape(1, 256, 64, 64))
            # Only mask0 is requested. Other mask tokens still participate in
            # attention exactly as upstream multimask_output=False requires.
            weights = self.decoder.output_hypernetworks_mlps[0](states[:, 1, :])
            logits = (weights.unsqueeze(1) @ upscaled.flatten(2)).reshape(1, 1, 256, 256)
            score = self.decoder.iou_prediction_head(states[:, 0, :])[:, 0:1]
            return {"mask_logits": logits, "score": score}

    return Encoder().eval(), Decoder().eval()


@contextmanager
def directory_fd(path):
    path = Path(path).absolute()
    held = []
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    try:
        current = os.open(path.anchor, flags)
        held.append(current)
        for part in path.parts[1:]:
            current = os.open(part, flags, dir_fd=current)
            held.append(current)
        yield current
    except OSError as error:
        raise verify.VerificationError("unsafe output parent") from error
    finally:
        for fd in reversed(held):
            os.close(fd)


def atomic_publish(parent_fd, staging_name, output_name):
    """Linux renameat2 RENAME_NOREPLACE: publish whole bundle without clobber."""
    verify.checked_relative(staging_name)
    verify.checked_relative(output_name)
    libc = ctypes.CDLL(None, use_errno=True)
    rename = getattr(libc, "renameat2", None)
    verify.require(rename is not None, "atomic no-replace rename unavailable")
    rename.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint)
    rename.restype = ctypes.c_int
    result = rename(parent_fd, staging_name.encode("ascii"), parent_fd, output_name.encode("ascii"), 1)
    if result != 0:
        code = ctypes.get_errno()
        raise verify.VerificationError("output already exists" if code == errno.EEXIST else "atomic publication failed")
    os.fsync(parent_fd)


def _write_at(directory, name, data):
    verify.checked_relative(name)
    verify.require(0 < len(data) <= verify.MAX_ARTIFACT, "converted artifact byte limit")
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW, 0o600, dir_fd=directory)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def _publish_with_tool_snapshot(parent, staging_name, output_name, tools_root, snapshot):
    revalidate_tool_snapshot(tools_root, snapshot)
    atomic_publish(parent, staging_name, output_name)


def convert(source_root, checkpoint, output):
    """Only entry point allowed to import torch/converter or construct models."""
    revalidate_tool_snapshot(TOOLS_ROOT, STARTUP_TOOL_SNAPSHOT)
    provenance = tool_provenance(STARTUP_TOOL_SNAPSHOT)
    catalog = conversion_catalog(STARTUP_TOOL_SNAPSHOT)
    sources = verify_source(source_root, catalog)
    checkpoint = Path(checkpoint).absolute()
    output = Path(output).absolute()
    verify.checked_relative(output.name)
    with verify.checked_open(checkpoint.parent, checkpoint.name) as (stream, info), directory_fd(output.parent) as parent:
        pin = catalog["model"]["checkpoint"]
        verify.require(info.st_size == pin["bytes"], "checkpoint size mismatch")
        digest = hashlib.sha256()
        remaining = pin["bytes"]
        while remaining:
            block = stream.read(min(65536, remaining))
            verify.require(bool(block), "short checkpoint")
            digest.update(block)
            remaining -= len(block)
        verify.require(stream.read(1) == b"", "checkpoint grew during verification")
        verify.require(digest.hexdigest() == pin["sha256"], "checkpoint hash mismatch")
        stream.seek(0)
        original_identity = (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        def unchanged_checkpoint():
            current = os.fstat(stream.fileno())
            verify.require(original_identity == (current.st_dev, current.st_ino, current.st_size, current.st_mtime_ns, current.st_ctime_ns), "checkpoint changed during conversion")
        unchanged_checkpoint()
        try:
            os.stat(output.name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise verify.VerificationError("output already exists")
        verify_versions(catalog)
        verify.require(not any(name == "mobile_sam" or name.startswith("mobile_sam.") for name in sys.modules), "MobileSAM already imported")
        staging_name = "." + output.name + ".staging-" + uuid.uuid4().hex
        os.mkdir(staging_name, mode=0o700, dir_fd=parent)
        finder = PinnedSource(sources, source_root)
        # No constructor receives a checkpoint path: upstream's unsafe load
        # branch is never used. Hashing and weights-only load share this fd.
        try:
            with offline_imports():
                torch = importlib.import_module("torch")
                verify_imported_cpu(torch, catalog)
                converter = importlib.import_module("litert_torch")
                torch.set_num_threads(2)
                torch.manual_seed(0)
                sys.meta_path.insert(0, finder)
                builder = importlib.import_module("mobile_sam.build_sam")
                sam = builder.build_sam_vit_t(checkpoint=None)
                unchanged_checkpoint()
                state = torch.load(stream, map_location="cpu", weights_only=True, mmap=False)
                unchanged_checkpoint()
                verify.require(type(state) in (dict, __import__("collections").OrderedDict) and all(type(key) is str and isinstance(value, torch.Tensor) for key, value in state.items()), "checkpoint is not a tensor state dictionary")
                sam.load_state_dict(state, strict=True)
                del state
                # TinyViT caches attention biases in eval(); rebuild the cache
                # AFTER loading trained biases instead of exporting random ones.
                sam.train(True)
                sam.eval()
                encoder, decoder = build_wrappers(torch, sam)
                samples = {
                    "encoder": {"image": torch.zeros((1, 3, 1024, 1024), dtype=torch.float32)},
                    "prompt_decoder": {
                        "image_embeddings": torch.zeros((1, 256, 64, 64), dtype=torch.float32),
                        "point_coords": torch.zeros((1, 66, 2), dtype=torch.float32),
                        "point_labels": torch.tensor([[1, -1] + [-1] * 64], dtype=torch.int32),
                        "point_valid": torch.tensor([[1, 1] + [0] * 64], dtype=torch.int32),
                        "token_count": torch.tensor([2], dtype=torch.int32),
                    },
                }
                artifacts = []
                with directory_fd(output.parent / staging_name) as staging:
                    for role, module in (("encoder", encoder), ("prompt_decoder", decoder)):
                        # No AOT/vendor target, Flex fallback, cloud call,
                        # quantizer or interpreter invocation is requested.
                        with torch.no_grad():
                            edge = converter.convert(module, sample_kwargs=samples[role], strict_export=True, enable_x64=False, runtime_constant_folding=False)
                        data = edge.model_content()
                        verify.require(type(data) is bytes and len(data) <= verify.MAX_ARTIFACT, "converter output size/type")
                        inspected = verify.inspect_tflite(data, role)
                        name = role + ".tflite"
                        _write_at(staging, name, data)
                        artifacts.append({"role": role, "path": name, "bytes": len(data), "sha256": verify.sha256(data), **inspected})
                        del edge, data
                    manifest = {"schema": "vw-assisted-model-v1", "status": "pending-admission", "model": "MobileSAM-vit_t", "source_commit": catalog["model"]["source_commit"], "checkpoint": {key: pin[key] for key in ("bytes", "sha256")}, "graph_contract": verify.GRAPH_CONTRACT, "artifacts": artifacts, "preprocessing": verify.PREPROCESSING, "postprocessing": verify.POSTPROCESSING, "converter": {"packages": catalog["converter"]["packages"], "files": provenance}}
                    raw = verify.canonical_bytes(manifest)
                    _write_at(staging, "model-manifest.json", raw)
                    os.fsync(staging)
                verify.verify_model(output.parent / staging_name, manifest, catalog)
                unchanged_checkpoint()
                _publish_with_tool_snapshot(parent, staging_name, output.name, TOOLS_ROOT, STARTUP_TOOL_SNAPSHOT)
                print("pending-admission modelArtifactSha256=" + verify.sha256(raw))
        finally:
            if finder in sys.meta_path:
                sys.meta_path.remove(finder)
            for name in list(sys.modules):
                if name == "mobile_sam" or name.startswith("mobile_sam."):
                    del sys.modules[name]
        # checked_open additionally refuses mutation of the bound checkpoint.


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    convert(args.source_root, args.checkpoint, args.output)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (verify.VerificationError, OSError, RuntimeError, ImportError) as error:
        print("offline conversion refused: " + str(error), file=sys.stderr)
        raise SystemExit(2) from None
