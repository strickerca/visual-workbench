"""Bind the image diagnostic APK to the source and license gate inputs."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]


def bindings():
    names = ["build.ps1", "tools/bench/image/build_receipt.py", "tools/check_gradle_licenses.py",
             "apps/build.gradle.kts", "apps/settings.gradle.kts", "apps/gradle/libs.versions.toml",
             "apps/gradle.properties", "tools/bench/image-android/build.gradle.kts",
             "tools/bench/image-android/gradle.lockfile",
             "tools/bench/image-android/build/outputs/apk/debug/image-bench-debug.apk"]
    paths = [ROOT / name for name in names] + [p for p in (ROOT / "tools/bench/image-android/src").rglob("*") if p.is_file()]
    return {p.relative_to(ROOT).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def main():
    current = {"schema": 1, "sha256": bindings()}
    path = ROOT / ".local/image-build.json"
    if sys.argv[1:] == ["record"]:
        path.write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8")
    elif sys.argv[1:] == ["check"]:
        if json.loads(path.read_text()) != current:
            raise ValueError("Image APK source binding differs; gated rebuild required")
    else:
        raise ValueError("Expected record or check")
    print("Image build binding: PASS")


if __name__ == "__main__":
    main()
