"""Fetch the pinned portable official Windows measurement CLI; no system install."""
import hashlib
import json
from pathlib import Path
import stat
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[3]
NAME = "vips-dev-x64-web-8.18.7.zip"
DIGEST = "3a122eb3d588690008216f786338e2f9329dba3c3c2768b5933cf7299d549241"
EXE_DIGEST = "713f52d657dcb041303eae9c34c05f323933fe4f201cde3fb4942936bc6b8727"
URL = "https://github.com/libvips/build-win64-mxe/releases/download/v8.18.7/" + NAME


def main():
    cache = ROOT / ".local/tools"
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / NAME
    if not archive.exists():
        with urllib.request.urlopen(URL, timeout=45) as response, archive.open("xb") as stream:
            copied = 0
            while data := response.read(1024 * 1024):
                copied += len(data)
                if copied > 11365048:
                    raise ValueError("Archive exceeded pinned size")
                stream.write(data)
                print(f"libvips archive: {copied}/11365048 bytes", flush=True)
    if archive.stat().st_size != 11365048 or hashlib.sha256(archive.read_bytes()).hexdigest() != DIGEST:
        raise ValueError("Archive checksum/size differs; preserved for diagnosis")
    destination = (cache / "libvips-8.18.7").resolve()
    executable = destination / "vips-dev-8.18/bin/vips.exe"
    if not destination.exists():
        with zipfile.ZipFile(archive) as source:
            for entry in source.infolist():
                path = (destination / entry.filename).resolve()
                if not path.is_relative_to(destination) or stat.S_ISLNK(entry.external_attr >> 16):
                    raise ValueError("Unsafe archive entry")
            source.extractall(destination)
    if hashlib.sha256(executable.read_bytes()).hexdigest() != EXE_DIGEST:
        raise ValueError("CLI checksum differs")
    # Bind the dynamic runtime as well as vips.exe; checking the loader alone
    # would not detect a changed libvips DLL in a reused cache.
    runtime = {}
    with zipfile.ZipFile(archive) as source:
        for entry in source.infolist():
            if entry.is_dir():
                continue
            path = (destination / entry.filename).resolve(strict=True)
            if not path.is_relative_to(destination) or path.is_symlink():
                raise ValueError("Extracted path containment changed")
            expected = hashlib.sha256(source.read(entry)).hexdigest()
            if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
                raise ValueError("Extracted tool bytes changed")
            if path.parent == executable.parent:
                runtime[entry.filename] = expected
    record = dict(schema=1, release_url="https://github.com/libvips/build-win64-mxe/releases/tag/v8.18.7",
                  archive=dict(name=NAME, size=11365048, digest="sha256:" + DIGEST, browser_download_url=URL),
                  executable=str(executable), executable_sha256=EXE_DIGEST, runtime_sha256=runtime)
    (ROOT / ".local/t009-vips-tool.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print("libvips 8.18.7 portable measurement tool checksum: PASS")


if __name__ == "__main__":
    main()
