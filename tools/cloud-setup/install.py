"""Pinned, task-local Linux bootstrap. Android acceptance requires an explicit flag."""
from __future__ import annotations

import argparse
import copy
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from urllib.request import urlopen
import xml.etree.ElementTree as ET
import zipfile

HERE = Path(__file__).resolve().parent
PINS = json.loads((HERE / "pins.json").read_text())
ROOT = Path(os.environ.get("VW_TOOLS_ROOT", "/workspace/.vw-tools"))
CACHE = ROOT / "downloads"
STATE = ROOT / "setup-state"


def run(command: list[str], *, cwd: Path | None = None, limit: int = 600) -> None:
    print("RUN", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, timeout=limit, check=True)


def digest(path: Path, algorithm: str = "sha256") -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, algorithm).hexdigest()


def download(url: str, expected: str, algorithm: str = "sha256") -> Path:
    path = CACHE / url.rsplit("/", 1)[-1]
    if not path.exists():
        print("DOWNLOAD", url, flush=True)
        partial = path.with_suffix(path.suffix + ".partial")
        with urlopen(url, timeout=60) as response, partial.open("wb") as out:
            shutil.copyfileobj(response, out, length=4 * 1024 * 1024)
        partial.replace(path)
    if digest(path, algorithm) != expected:
        raise SystemExit(f"Checksum mismatch: {path.name}; preserved for diagnosis")
    return path


def prepare_environment() -> None:
    os.environ.update({
        "CARGO_HOME": str(ROOT / "cargo"), "RUSTUP_HOME": str(ROOT / "rustup"),
        "GRADLE_USER_HOME": str(ROOT / "gradle"), "npm_config_cache": str(ROOT / "npm-cache"),
        "ANDROID_USER_HOME": str(ROOT / "android-user"),
        "ANDROID_HOME": str(ROOT / "android-sdk"), "ANDROID_SDK_ROOT": str(ROOT / "android-sdk"),
        "JAVA_HOME": str(ROOT / "jdk-root/usr/lib/jvm/java-21-openjdk-amd64"),
        "CARGO_BUILD_JOBS": "2",
    })
    additions = [ROOT / "node-v24.21.0-linux-x64/bin", ROOT / "cargo/bin",
                 Path(os.environ["JAVA_HOME"]) / "bin"]
    os.environ["PATH"] = os.pathsep.join(map(str, additions)) + os.pathsep + os.environ["PATH"]
    for path in (CACHE, STATE, ROOT / "android-user", ROOT / "gradle", ROOT / "npm-cache"):
        path.mkdir(parents=True, exist_ok=True)


def install_core() -> None:
    # The audited base contains rustup. Do not silently add an untested system installer.
    rustup = ROOT / "cargo/bin/rustup"
    if not rustup.is_file():
        raise SystemExit("Audited rustup base is absent; select the published base with /workspace/.vw-tools/cargo/bin/rustup")
    for item in PINS["archives"]:
        name = item["name"]
        if name == "node":
            if not (ROOT / item["directory"] / "bin/node").exists():
                archive = download(item["url"], item["sha256"])
                with tarfile.open(archive) as bundle:
                    bundle.extractall(ROOT, filter="data")
        elif name == "jdk":
            jdk = Path(os.environ["JAVA_HOME"])
            if (jdk / "bin/javac").exists():
                continue
            base = Path("/usr/lib/jvm/java-21-openjdk-amd64")
            version = subprocess.check_output([str(base / "bin/java"), "-version"],
                                              stderr=subprocess.STDOUT, text=True)
            if 'version "21.0.12.1"' not in version:
                raise SystemExit("Base Java differs from audited JDK compiler package")
            archive = download(item["url"], item["sha256"])
            if not jdk.exists():
                shutil.copytree(base, jdk, symlinks=True)
            run(["dpkg-deb", "-x", str(archive), str(ROOT / "jdk-root")])
        else:
            archive = download(item["url"], item["sha256"])
            extracted = CACHE / (archive.name + ".contents")
            if not extracted.exists():
                with tarfile.open(archive) as bundle:
                    bundle.extractall(extracted, filter="data")
            for binary in item["binaries"]:
                source = next(path for path in extracted.rglob(binary) if path.is_file())
                destination = ROOT / "cargo/bin" / binary
                if not destination.exists() or digest(source) != digest(destination):
                    shutil.copy2(source, destination)
                    destination.chmod(0o755)
    run([str(rustup), "toolchain", "install", "1.99.0", "--profile", "minimal",
         "--component", "rustfmt", "--component", "clippy"])
    run([str(rustup), "target", "add", "--toolchain", "1.99.0",
         "aarch64-linux-android", "x86_64-pc-windows-msvc"])


def install_android(approved: bool) -> None:
    if not approved:
        raise SystemExit("Android requires --accept-android-sdk-license after explicit owner approval")
    license_pin = PINS["android_license"]
    with urlopen(license_pin["repository_url"], timeout=60) as response:
        metadata = response.read()
    repository = ET.fromstring(metadata)
    namespaces = dict(value for _, value in ET.iterparse(io.BytesIO(metadata), events=("start-ns",)))
    element = next(node for node in repository.findall("license")
                   if node.get("id") == license_pin["id"])
    terms = "".join(element.itertext()).strip()
    if hashlib.sha256(terms.encode()).hexdigest() != license_pin["package_text_sha256"]:
        raise SystemExit("Google package license text changed; renewed review required")
    if hashlib.sha1(terms.encode()).hexdigest() != license_pin["sdkmanager_sha1"]:
        raise SystemExit("SDK package license hash mismatch")
    sdk = ROOT / "android-sdk"
    # Record only the exact approved SDK agreement, not every remote SDK license.
    licenses = sdk / "licenses"
    licenses.mkdir(parents=True, exist_ok=True)
    license_file = licenses / license_pin["id"]
    accepted = set(license_file.read_text().splitlines()) if license_file.exists() else set()
    accepted.add(license_pin["sdkmanager_sha1"])
    license_file.write_text("\n".join(sorted(accepted)) + "\n")
    (STATE / "accepted-android-sdk-terms.txt").write_text(terms + "\n")
    receipt_path = STATE / "android-license-acceptance.json"
    if not receipt_path.exists():
        receipt_path.write_text(json.dumps({
            **license_pin, "accepted_by": "explicit installer flag following owner approval",
            "accepted_at_utc": datetime.now(timezone.utc).isoformat(),
        }, indent=2) + "\n")

    def register_package(item: dict, destination: Path) -> None:
        # Official zip payloads need the local metadata sdkmanager normally adds.
        remote = next(node for node in repository.findall("remotePackage")
                      if node.get("path") == item["package"])
        revision = ".".join(node.text for node in remote.find("revision"))
        if revision != item["revision"]:
            raise SystemExit(f"Remote SDK revision changed: {item['package']}")
        common = "http://schemas.android.com/repository/android/common/02"
        ET.register_namespace("common", common)
        ET.register_namespace("xsi", "http://www.w3.org/2001/XMLSchema-instance")
        local_root = ET.Element("{" + common + "}repository")
        local_root.append(copy.deepcopy(element))
        local = ET.SubElement(local_root, "localPackage", path=item["package"], obsolete="false")
        for child in remote:
            if child.tag not in ("archives", "channelRef"):
                local.append(copy.deepcopy(child))
        for node in local.iter():
            type_name = node.get("{http://www.w3.org/2001/XMLSchema-instance}type", "")
            if ":" in type_name:
                prefix = type_name.split(":", 1)[0]
                local_root.set("xmlns:" + prefix, namespaces[prefix])
        data = ET.tostring(local_root, encoding="utf-8", xml_declaration=True)
        package_file = destination / "package.xml"
        if not package_file.exists() or package_file.read_bytes() != data:
            package_file.write_bytes(data)

    for item in PINS["android"]:
        destination = sdk / item["directory"]
        properties = destination / "source.properties"
        if properties.exists():
            values = dict(tuple(part.strip() for part in line.split("=", 1))
                          for line in properties.read_text().splitlines()
                          if "=" in line and not line.lstrip().startswith("#"))
            if values.get("Pkg.Revision", "").strip() == item["revision"]:
                register_package(item, destination)
                print("PRESENT", item["package"], item["revision"], flush=True)
                continue
            raise SystemExit(f"Existing Android package revision differs: {item['package']}")
        archive = download(item["url"], item["sha1"], "sha1")
        with tempfile.TemporaryDirectory(prefix="android-extract-", dir=STATE) as temporary:
            extracted = Path(temporary)
            with zipfile.ZipFile(archive) as bundle:
                for member in bundle.infolist():
                    path = Path(member.filename)
                    if path.is_absolute() or ".." in path.parts:
                        raise SystemExit("Unsafe official archive path")
            # unzip preserves the NDK's executable modes and tool symlinks.
            run(["unzip", "-q", str(archive), "-d", str(extracted)], limit=300)
            children = list(extracted.iterdir())
            source = extracted / item["archive_root"] if "archive_root" in item else children[0]
            if not source.is_dir() or ("archive_root" not in item and len(children) != 1):
                raise SystemExit(f"Unexpected archive root: {archive.name}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(source), destination)
        register_package(item, destination)
        print("INSTALLED", item["package"], item["revision"], flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--android", action="store_true")
    parser.add_argument("--accept-android-sdk-license", action="store_true")
    parser.add_argument("--repo", type=Path, default=HERE.parents[1])
    args = parser.parse_args()
    repo = args.repo.resolve()
    rust = tomllib.loads((repo / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    node = json.loads((repo / "mcp/package.json").read_text())["engines"]["node"]
    if rust != PINS["rust"] or node != PINS["node"]:
        raise SystemExit("Source toolchain pins changed; update setup pins after review")
    prepare_environment()
    install_core()
    if args.android:
        install_android(args.accept_android_sdk_license)
    run(["python3", str(HERE / "proxy.py")])
    run(["cargo", "fetch", "--locked"], cwd=repo)
    run(["npm", "--prefix", str(repo / "mcp"), "ci", "--ignore-scripts", "--no-audit", "--no-fund"], cwd=repo)
    run(["bash", str(repo / "apps/gradlew"), "-p", str(repo / "apps"), "help",
         "--console=plain", "--no-daemon", "--no-parallel", "--max-workers=2"], cwd=repo)
    print("SETUP COMPLETE: task-local tools/caches; published environment unchanged", flush=True)


if __name__ == "__main__":
    main()
