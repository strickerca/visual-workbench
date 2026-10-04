"""Populate a fresh owned server stage after exact production npm admission."""
from pathlib import Path
import json
import sys
from check_npm_licenses import ROOT, checked, verify


def stage(work: Path, node: Path) -> None:
    work = work.resolve(strict=True)
    if not work.is_relative_to((ROOT / ".local/mcp-build").resolve()):
        raise ValueError("stage_owner")
    destination = work / "stage"
    verify(ROOT, installed=destination / "mcp", production=True, node=node)

    def copy(source: Path, relative: str) -> None:
        target = destination / relative
        data = checked(source.parent, source.name, 256 * 1024 * 1024)
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open("xb") as output:
            output.write(data)

    for name in ("vw-mcp-host.exe", "vw-mcp-package.exe", "vw-codex-host.exe"):
        copy(ROOT / "target/debug" / name, name)
    copy(node / "node.exe", "node.exe")
    copy(node / "LICENSE", "notices/Node-LICENSE.txt")
    for path in sorted((ROOT / "mcp/src").rglob("*.mjs")):
        copy(path, path.relative_to(ROOT).as_posix())
    for name in ("main.mjs", "client.mjs", "package.mjs", "process.mjs", "schema.mjs",
                 "schema-worker.mjs", "image-profiles.json"):
        copy(ROOT / "mcp/codex" / name, "mcp/codex/" + name)
    policy = json.loads(checked(ROOT, "tools/licenses/reviewed-npm.json"))
    for row in policy["packages"]:
        if row["dev"]:
            continue
        for notice in row["notices"]:
            copy(ROOT / "third_party" / notice["retained"], notice["retained"])
    # SDK documentation has a separate content license. Repository metadata is
    # also excluded before hashing: Gradle's default resource copy excludes VCS
    # files, which otherwise leaves the packaged runtime inventory incomplete.
    # Keep runtime code, package metadata and complete legal notices.
    for path in (destination / "mcp/node_modules").rglob("*"):
        parts = path.relative_to(destination / "mcp/node_modules").parts
        optional = path.name.lower().startswith("readme") or ".github" in parts or path.name in {
            ".gitattributes", ".gitignore", ".gitmodules", ".npmignore",
        }
        if path.is_file() and optional:
            if not path.resolve().is_relative_to(destination.resolve()) or path.is_symlink():
                raise ValueError("readme_owner")
            path.unlink()
    print("MCP_STAGE production_dependencies=9 node=24.21.0 scripts=disabled")


if __name__ == "__main__":
    stage(Path(sys.argv[1]), Path(sys.argv[2]))
