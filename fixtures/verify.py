"""Verify exact corpus bytes without decoding hostile fixtures."""
import argparse
import hashlib
import json
from pathlib import Path


def verify(directory: Path, manifest: dict) -> int:
    directory = directory.resolve(strict=True)
    records = manifest.get("files")
    if not isinstance(records, list) or not records:
        raise ValueError("Empty fixture manifest")
    names = set()
    for record in records:
        name = record["path"]
        if not isinstance(name, str) or Path(name).name != name or "/" in name or "\\" in name or ":" in name or name in names:
            raise ValueError("Duplicate or escaping fixture path")
        names.add(name)
        path = (directory / name).resolve(strict=True)
        if path.parent != directory or not path.is_file() or path.stat().st_size != record["bytes"]:
            raise ValueError("Fixture path or size changed")
        if not record.get("provenance") or not record.get("license"):
            raise ValueError("Missing fixture provenance/license")
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != record["sha256"]:
            raise ValueError("Fixture hash changed")
    return len(names)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    count = verify(args.directory, json.loads(args.manifest.read_text(encoding="utf-8")))
    print(f"Verified {count} fixture hashes/provenance lines; hostile files not decoded")
    print("SKIP real owner photo: no sanitized private fixture is bound by this public corpus verifier")
