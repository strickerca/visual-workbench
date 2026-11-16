#!/usr/bin/env python3
"""Fail when a tracked Markdown file links to a repository file that does not exist.

Only relative Markdown links are checked. Vendored and upstream notice text is
skipped because it is copied byte for byte.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKIP = ("third_party/", "drivers/sudovda/vendor/", "docs/license-proposals/notices/", "fixtures/")
LINK = re.compile(r"\[[^\]\n]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")


def main() -> int:
    tracked = subprocess.run(["git", "ls-files", "*.md"], cwd=ROOT, capture_output=True, text=True, check=True)
    problems = []
    checked = 0
    for name in tracked.stdout.splitlines():
        path = ROOT / name
        if name.startswith(SKIP) or not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for number, line in enumerate(text.splitlines(), 1):
            for target in LINK.findall(line):
                if re.match(r"[a-z][a-z0-9+.-]*:", target, re.I) or target.startswith("#"):
                    continue
                target = target.split("#", 1)[0]
                if not target:
                    continue
                checked += 1
                resolved = (ROOT / target.lstrip("/")) if target.startswith("/") else (path.parent / target)
                if not resolved.exists():
                    problems.append(f"{name}:{number}: missing {target}")
    for problem in problems:
        print(problem)
    print(f"DOC LINKS: {'FAIL' if problems else 'OK'} ({checked} relative links checked)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
