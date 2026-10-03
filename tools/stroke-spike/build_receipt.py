"""Bind both comparison APKs and native runners to exact build inputs."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def bindings():
    names = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "build.ps1",
             "tools/stroke-spike/build_receipt.py", "tools/stroke-spike/hil-stroke.ps1", "tools/check_gradle_licenses.py",
             "apps/build.gradle.kts", "apps/settings.gradle.kts", "apps/gradle/libs.versions.toml",
             "apps/gradle.properties", "tools/stroke-spike/android/build.gradle.kts", "tools/stroke-spike/android/gradle.lockfile",
             "target/release/vw-stroke-spike.exe",
             "target/aarch64-linux-android/release/vw-stroke-spike",
             "target/aarch64-linux-android/release/libvw_stroke_jni.so",
             "tools/stroke-spike/android/build/outputs/apk/debug/stroke-spike-debug.apk",
             "tools/stroke-spike/android/build/outputs/apk/androidTest/debug/stroke-spike-debug-androidTest.apk"]
    paths = {ROOT / name for name in names}
    for directory in ("core/crates/vw-ink", "core/crates/vw-model", "core/crates/vw-geom", "core/crates/vw-proto", "contracts",
                      "tools/stroke-spike/native", "tools/stroke-spike/src", "tools/stroke-spike/android/src"):
        paths.update(p for p in (ROOT / directory).rglob("*") if p.is_file())
    paths.add(ROOT / "tools/stroke-spike/Cargo.toml")
    return {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(paths)}


def main():
    current = {"schema": 1, "sha256": bindings()}
    path = ROOT / ".local/stroke-build.json"
    if sys.argv[1:] == ["record"]:
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8")
    elif sys.argv[1:] == ["check"]:
        if json.loads(path.read_text(encoding="utf-8")) != current:
            raise ValueError("Stroke APK or source changed; gated rebuild required")
    else:
        raise ValueError("Expected record or check")
    print("Stroke build binding: PASS")


if __name__ == "__main__":
    main()
