#!/usr/bin/env python3
"""Bounded, extraction-free native inventory of one final arm64 Android APK.

Usage: python tools/ffi-test/check_apk.py app.apk [--require libextra.so]
Prints one text-only JSON receipt; exits 1 on refusal. APK signing-block gaps are
allowed, but signatures are not authenticated here. ZIP64/multidisk archives are
outside this deliberately bounded APK32 contract. No runtime acceptance is made.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
from dataclasses import dataclass
from typing import BinaryIO
import zlib


ALIGNMENT = 16_384
CHUNK = 1024 * 1024
REQUIRED = frozenset(("libvw_core.so", "libjnidispatch.so"))
LIBRARY_NAME = re.compile(r"lib[A-Za-z0-9_.+-]+\.so\Z")


@dataclass(frozen=True)
class Limits:
    max_apk_bytes: int = 2 * 1024**3
    max_entries: int = 65_534
    max_metadata_bytes: int = 32 * 1024**2
    max_name_bytes: int = 1024
    max_libraries: int = 128
    max_library_bytes: int = 512 * 1024**2
    max_program_headers: int = 128
    max_load_memory_bytes: int = 1024**3


class Rejected(ValueError):
    """Stable, non-sensitive refusal code; never includes a host path."""


@dataclass(frozen=True)
class Entry:
    name: str
    raw_name: bytes
    flags: int
    method: int
    crc: int
    compressed: int
    size: int
    offset: int
    directory: bool
    library: bool


def require(condition: bool, code: str) -> None:
    if not condition:
        raise Rejected(code)


def read_at(stream: BinaryIO, offset: int, length: int, end: int) -> bytes:
    require(0 <= offset <= end and 0 <= length <= end - offset, "truncated_range")
    stream.seek(offset)
    data = stream.read(length)
    require(len(data) == length, "truncated_read")
    return data


def directory(stream: BinaryIO, size: int, limits: Limits) -> tuple[list[Entry], int]:
    tail_at = max(0, size - 65_557)
    tail = read_at(stream, tail_at, size - tail_at, size)
    position = tail.rfind(b"PK\x05\x06")
    while position >= 0:
        if (len(tail) - position >= 22 and
                position + 22 + struct.unpack_from("<H", tail, position + 20)[0] == len(tail)):
            break
        position = tail.rfind(b"PK\x05\x06", 0, position)
    require(position >= 0, "missing_eocd")
    record = struct.unpack_from("<4s4H2IH", tail, position)
    _, disk, cd_disk, disk_count, count, cd_size, cd_at, comment = record
    eocd_at = tail_at + position
    require(position + 22 + comment == len(tail), "trailing_or_truncated_zip")
    require(disk == cd_disk == 0 and disk_count == count, "multidisk_zip")
    require(count != 0xFFFF and cd_size != 0xFFFFFFFF and cd_at != 0xFFFFFFFF, "zip64_unsupported")
    if eocd_at >= 20:
        require(read_at(stream, eocd_at - 20, 4, size) != b"PK\x06\x07", "zip64_unsupported")
    require(0 < count <= limits.max_entries, "entry_limit")
    require(cd_size + 22 + comment <= limits.max_metadata_bytes, "metadata_limit")
    require(cd_at + cd_size == eocd_at, "central_directory_range")
    require(count * 46 <= cd_size, "central_directory_count")
    entries: list[Entry] = []
    seen: set[str] = set()
    cursor = cd_at
    for _ in range(count):
        values = struct.unpack("<4s6H3I5H2I", read_at(stream, cursor, 46, eocd_at))
        (signature, _made, needed, flags, method, _time, _date, crc, compressed,
         unpacked, name_len, extra_len, comment_len, start_disk, _internal,
         external, local_at) = values
        require(signature == b"PK\x01\x02", "central_directory_signature")
        require(needed <= 20 and start_disk == 0 and
                0xFFFFFFFF not in (compressed, unpacked, local_at), "unsupported_zip_feature")
        require(flags & ~0x080E == 0, "encrypted_or_unsupported_flags")
        require(method in (0, 8), "unsupported_compression")
        require(0 < name_len <= limits.max_name_bytes, "entry_name_limit")
        length = 46 + name_len + extra_len + comment_len
        require(cursor + length <= eocd_at, "central_directory_range")
        raw_name = read_at(stream, cursor + 46, name_len, eocd_at)
        try:
            name = raw_name.decode("utf-8" if flags & 0x800 else "cp437")
        except UnicodeError as error:
            raise Rejected("entry_name_encoding") from error
        is_directory = name.endswith("/")
        clean_name = name[:-1] if is_directory else name
        parts = clean_name.split("/")
        require(all(part not in ("", ".", "..") for part in parts) and
                "\\" not in name and ":" not in name and
                not any(ord(c) < 32 or ord(c) == 127 for c in name), "unsafe_entry_name")
        require(clean_name not in seen, "duplicate_entry")
        seen.add(clean_name)
        kind = stat.S_IFMT(external >> 16)
        require(kind in (0, stat.S_IFDIR if is_directory else stat.S_IFREG), "nonregular_entry")
        library = not is_directory and (parts[0] == "lib" or name.lower().endswith(".so"))
        if parts[0] == "lib":
            require(len(parts) == 1 or parts[1] == "arm64-v8a", "unexpected_abi")
        if library:
            require(len(parts) == 3 and parts[:2] == ["lib", "arm64-v8a"] and
                    LIBRARY_NAME.fullmatch(parts[2]) is not None, "unexpected_native_path")
            require(method == 0 and compressed == unpacked, "compressed_native_library")
            require(64 <= unpacked <= limits.max_library_bytes, "library_size_limit")
        if method == 0:
            require(compressed == unpacked, "stored_size_mismatch")
        if is_directory:
            require(compressed == unpacked == 0, "nonempty_directory")
        entries.append(Entry(name, raw_name, flags, method, crc, compressed,
                             unpacked, local_at, is_directory, library))
        cursor += length
    require(cursor == eocd_at, "central_directory_size")
    return entries, cd_at


def local_ranges(stream: BinaryIO, entries: list[Entry], cd_at: int,
                 limits: Limits) -> dict[str, int]:
    ranges: list[tuple[int, int]] = []
    positions: dict[str, int] = {}
    metadata = 0
    for entry in entries:
        fields = struct.unpack("<4s5H3I2H", read_at(stream, entry.offset, 30, cd_at))
        signature, needed, flags, method, _time, _date, crc, packed, size, names, extra = fields
        require(signature == b"PK\x03\x04" and needed <= 20, "local_header_signature")
        require(flags == entry.flags and method == entry.method and names == len(entry.raw_name),
                "local_header_mismatch")
        metadata += 30 + names + extra
        require(metadata <= limits.max_metadata_bytes, "local_metadata_limit")
        require(read_at(stream, entry.offset + 30, names, cd_at) == entry.raw_name, "local_name_mismatch")
        data_at = entry.offset + 30 + names + extra
        data_end = data_at + entry.compressed
        require(data_at <= cd_at and data_end <= cd_at, "entry_data_range")
        if flags & 8:
            require(crc in (0, entry.crc) and packed in (0, entry.compressed) and
                    size in (0, entry.size), "local_descriptor_mismatch")
            expected = (entry.crc, entry.compressed, entry.size)
            raw = read_at(stream, data_end, 12, cd_at)
            if struct.unpack("<3I", raw) == expected:
                data_end += 12
            else:
                require(raw[:4] == b"PK\x07\x08" and
                        struct.unpack("<3I", read_at(stream, data_end + 4, 12, cd_at)) == expected,
                        "data_descriptor_mismatch")
                data_end += 16
        else:
            require((crc, packed, size) == (entry.crc, entry.compressed, entry.size), "local_size_crc_mismatch")
        if entry.library:
            require(data_at % ALIGNMENT == 0, "zip_native_alignment")
            positions[entry.name] = data_at
        ranges.append((entry.offset, data_end))
    previous = 0
    for start, end in sorted(ranges):
        require(start >= previous, "overlapping_zip_entries")
        previous = end
    return positions


def elf_headers(stream: BinaryIO, data_at: int, size: int, limits: Limits) -> list[dict[str, int]]:
    end = data_at + size
    values = struct.unpack("<16sHHIQQQIHHHHHH", read_at(stream, data_at, 64, end))
    (identity, kind, machine, version, _entry, ph_at, _sh_at, _flags, eh_size,
     ph_size, ph_count, _sh_size, _sh_count, _sh_names) = values
    require(identity[:7] == b"\x7fELF\x02\x01\x01" and kind == 3 and machine == 183 and
            version == 1 and eh_size == 64, "elf_arm64_shared_header")
    require(ph_size == 56 and 0 < ph_count <= limits.max_program_headers and
            ph_at >= 64 and ph_at % 8 == 0 and ph_at + ph_count * ph_size <= size,
            "elf_program_header_bounds")
    loads: list[dict[str, int]] = []
    reserved = 0
    for index in range(ph_count):
        fields = struct.unpack("<II6Q", read_at(stream, data_at + ph_at + index * 56, 56, end))
        kind, flags, offset, address, _physical, file_bytes, memory_bytes, align = fields
        require(offset <= size and file_bytes <= size - offset, "elf_segment_file_range")
        if kind != 1:
            continue
        require(flags & ~7 == 0 and file_bytes <= memory_bytes and
                address + memory_bytes < 2**64, "elf_load_range")
        require(align >= ALIGNMENT and align & (align - 1) == 0, "elf_load_alignment")
        require(offset % align == address % align, "elf_load_congruence")
        require(not loads or address >= loads[-1]["virtual_address"], "elf_load_order")
        reserved += memory_bytes
        require(reserved <= limits.max_load_memory_bytes, "elf_load_memory_limit")
        loads.append({"offset": offset, "virtual_address": address, "file_bytes": file_bytes,
                      "memory_bytes": memory_bytes, "alignment": align, "flags": flags})
    require(bool(loads), "elf_missing_load")
    return loads


def hash_region(stream: BinaryIO, offset: int, size: int) -> tuple[str, int]:
    stream.seek(offset)
    digest = hashlib.sha256()
    checksum = 0
    remaining = size
    while remaining:
        data = stream.read(min(CHUNK, remaining))
        require(bool(data), "truncated_hash_read")
        remaining -= len(data)
        digest.update(data)
        checksum = zlib.crc32(data, checksum)
    return digest.hexdigest(), checksum


def verify_apk(path: str | os.PathLike[str], *, additional_required: tuple[str, ...] = (),
               limits: Limits = Limits()) -> dict[str, object]:
    require(all(isinstance(value, int) and value > 0 for value in vars(limits).values()), "invalid_limits")
    require(all(LIBRARY_NAME.fullmatch(name) is not None for name in additional_required), "invalid_required_library")
    with Path(path).open("rb") as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode), "apk_not_regular")
        require(22 <= before.st_size <= limits.max_apk_bytes, "apk_size_limit")
        entries, cd_at = directory(stream, before.st_size, limits)
        natives = [entry for entry in entries if entry.library]
        require(0 < len(natives) <= limits.max_libraries, "native_inventory_limit")
        names = {entry.name.rsplit("/", 1)[1] for entry in natives}
        required = REQUIRED | set(additional_required)
        require(required <= names, "required_library_missing")
        positions = local_ranges(stream, entries, cd_at, limits)
        libraries = []
        for entry in sorted(natives, key=lambda value: value.name):
            offset = positions[entry.name]
            loads = elf_headers(stream, offset, entry.size, limits)
            digest, checksum = hash_region(stream, offset, entry.size)
            require(checksum == entry.crc, "native_crc_mismatch")
            libraries.append({"entry": entry.name, "bytes": entry.size, "sha256": digest,
                              "zip_method": "stored", "zip_data_offset": offset,
                              "zip_alignment_bytes": ALIGNMENT, "zip_alignment_remainder": offset % ALIGNMENT,
                              "crc32": f"{checksum:08x}", "elf_machine": "AArch64",
                              "elf_class": 64, "elf_type": "ET_DYN", "loads": loads})
        digest, _ = hash_region(stream, 0, before.st_size)
        after = os.fstat(stream.fileno())
        require((before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                (after.st_size, after.st_mtime_ns, after.st_ctime_ns), "apk_changed_during_inspection")
    return {"schema": "vw-apk-native-inventory-v1", "status": "PASS",
            "apk_sha256": digest, "apk_bytes": before.st_size, "zip_entries": len(entries),
            "required_libraries": sorted(required), "libraries": libraries,
            "limits": vars(limits), "extracted_files": 0, "runtime_16k_verified": False,
            "signature_verified": False}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    parser.add_argument("--require", action="append", default=[], metavar="LIBRARY.so",
                        help="additional required arm64 library; baseline requirements always apply")
    args = parser.parse_args()
    try:
        report = verify_apk(args.apk, additional_required=tuple(args.require))
    except (Rejected, OSError, struct.error, OverflowError) as error:
        print(json.dumps({"schema": "vw-apk-native-inventory-v1", "status": "FAIL",
                          "error": str(error) if isinstance(error, Rejected) else "apk_read_failure",
                          "runtime_16k_verified": False, "signature_verified": False}, sort_keys=True))
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
