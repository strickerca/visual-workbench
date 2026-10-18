"""Reuse existing proxy access for Java; never record credentials in properties."""
import os
from pathlib import Path
from urllib.parse import urlsplit


def configure() -> None:
    root = Path(os.environ["GRADLE_USER_HOME"])
    root.mkdir(parents=True, exist_ok=True)
    path = root / "gradle.properties"
    marker = "# Visual Workbench task proxy settings"
    original = path.read_text() if path.exists() else ""
    prefixes = tuple(
        f"systemProp.{scheme}.proxy{part}="
        for scheme in ("http", "https") for part in ("Host", "Port")
    )
    lines = [line for line in original.splitlines()
             if not line.startswith(prefixes) and line != marker]
    lines.append(marker)
    for scheme in ("http", "https"):
        value = os.environ.get(f"{scheme.upper()}_PROXY") or os.environ.get(f"{scheme}_proxy")
        if not value:
            continue
        uri = urlsplit(value)
        if uri.username or uri.password or not uri.hostname:
            raise SystemExit("Invalid/authenticated proxy: approved Java configuration required")
        lines.extend((f"systemProp.{scheme}.proxyHost={uri.hostname}",
                      f"systemProp.{scheme}.proxyPort={uri.port or 80}"))
    path.write_text("\n".join(lines) + "\n")
    path.chmod(0o600)


if __name__ == "__main__":
    configure()
