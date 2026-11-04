"""Small synthetic, stdlib-only checks for LIC-003 notice generation.

No cargo, Gradle, npm, network or real package caches are used; every graph,
archive, toolchain and license file below is a fixture built in a temporary
directory.
"""
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import sys
import tarfile
import tempfile
import unittest
import zipfile

import generate_notices as g

MIT = b"Copyright (c) Fixture Authors\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\n" \
      b"... The above copyright notice and this permission notice shall be included ...\n"
APACHE = b"                                 Apache License\n                           Version 2.0, January 2004\n" \
         b"   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION\n"
APACHE_POINTER = b"Licensed under the Apache License, Version 2.0; see https://www.apache.org/licenses/LICENSE-2.0\n"
BSD = b"Copyright (c) Fixture\nRedistribution and use in source and binary forms, with or without modification,\n" \
      b"1. Redistributions of source code must retain the above copyright notice\n" \
      b"2. Redistributions in binary form must reproduce the above copyright notice\n" \
      b"* Neither the name of the copyright holder nor the names of its contributors\n"
GPL = b"                    GNU GENERAL PUBLIC LICENSE\n                       Version 2, June 1991\n" \
      b"    TERMS AND CONDITIONS FOR COPYING, DISTRIBUTION AND MODIFICATION\n"
MPL = b"Mozilla Public License Version 2.0\n==================================\n" \
      b"Exhibit A - Source Code Form License Notice\n"
BOOST = b"Boost Software License - Version 1.0\nPermission is hereby granted, free of charge, to any person or " \
        b"organization obtaining a copy of the software\nThe copyright notices in the Software and this entire statement\n"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def crate(prefix: str, files: dict) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w") as tar:
        for name, body in sorted(files.items()):
            info = tarfile.TarInfo(prefix + "/" + name)
            info.size = len(body)
            tar.addfile(info, io.BytesIO(body))
    return gzip.compress(raw.getvalue(), mtime=0)


def jar(files: dict) -> bytes:
    raw = io.BytesIO()
    with zipfile.ZipFile(raw, "w") as archive:
        for name, body in sorted(files.items()):
            archive.writestr(zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)), body)
    return raw.getvalue()


class Spdx(unittest.TestCase):
    def test_expressions_become_disjunctive_normal_form(self):
        self.assertEqual(g.spdx_dnf("MIT OR Apache-2.0"), [("MIT",), ("Apache-2.0",)])
        self.assertEqual(g.spdx_dnf("MIT/Apache-2.0"), [("MIT",), ("Apache-2.0",)])
        self.assertEqual(g.spdx_dnf("(MIT OR Apache-2.0) AND Unicode-3.0"),
                         [("MIT", "Unicode-3.0"), ("Apache-2.0", "Unicode-3.0")])
        self.assertEqual(g.spdx_dnf("Apache-2.0 WITH LLVM-exception OR MIT"),
                         [("Apache-2.0 WITH LLVM-exception",), ("MIT",)])
        for bad in ("MIT OR", "(MIT", "MIT AND AND ISC", "MIT; ISC", "", "MIT\nISC"):
            with self.subTest(bad=bad):
                with self.assertRaises(g.Refused):
                    g.spdx_dnf(bad)

    def test_detection_and_file_names(self):
        self.assertEqual(g.detect(MIT), {"mit"})
        self.assertEqual(g.detect(APACHE), {"apache"})
        self.assertEqual(g.detect(APACHE_POINTER), frozenset())  # pointer, not the license text
        self.assertEqual(g.detect(BSD), {"bsd"})
        self.assertEqual(g.detect(GPL), {"gpl"})
        self.assertEqual(g.family("MIT-Modern-Variant"), "mit-modern")
        self.assertEqual(g.detect(b"Dual licensed, see the other files"), frozenset())
        for name in ("LICENSE", "LICENSE-MIT", "COPYING", "license.md", "NOTICE.txt", "UNLICENSE", "PATENTS",
                     "META-INF/LICENSE.txt", "MIT-LICENSE", "THIRD_PARTY_NOTICE.txt"):
            self.assertTrue(g.is_license_file(name), name)
        for name in ("license.rs", "README.md", "LicenseCheck.class", "src/licenses.json", "x.kotlin_module"):
            self.assertFalse(g.is_license_file(name), name)

    def test_every_family_needs_full_text_markers(self):
        for pointer in (b"This crate is licensed under the Mozilla Public License 2.0 (MPL-2.0).\n",
                        b"Distributed under the Boost Software License, Version 1.0.\n",
                        b"Unicode-3.0: see the Unicode license at https://www.unicode.org/license.txt\n",
                        b"Licensed under the SIL Open Font License 1.1.\n",
                        b"Dedicated to the public domain under CC0 1.0.\n",
                        b"zlib license: this software is provided as-is.\n",
                        b"Redistribution and use in source and binary forms are permitted (see BSD).\n"):
            with self.subTest(pointer=pointer):
                self.assertEqual(g.detect(pointer), frozenset())
        self.assertEqual(g.detect(MPL), {"mpl"})
        self.assertEqual(g.detect(BOOST), {"bsl"})  # the Boost grant is not MIT
        zlib = b"This software is provided 'as-is', without any express or implied warranty.\n" \
               b"2. Altered source versions must be plainly marked as such\n"
        self.assertEqual(g.detect(zlib), {"zlib"})


class Selection(unittest.TestCase):
    def builder(self):
        return g.Builder({"Apache-2.0": ("canonical:Apache-2.0:fixture", APACHE)})

    def only(self, builder):
        self.assertEqual(len(builder.components), 1)
        return next(iter(builder.components.values()))

    def test_choose_a_branch_whose_text_the_package_ships(self):
        b = self.builder()
        b.add("cargo", "a", "1.0.0", "MIT OR Apache-2.0", g.spdx_dnf("MIT OR Apache-2.0"),
              [("package:LICENSE-APACHE", APACHE)], "rust:t")
        row = self.only(b)
        self.assertEqual(row["selected"], "Apache-2.0")
        self.assertEqual(row["texts"], [{"origin": "package:LICENSE-APACHE", "sha256": sha(APACHE)}])
        self.assertEqual(row["warnings"], [])

    def test_pointer_or_notice_files_never_count_as_license_text(self):
        b = self.builder()
        b.add("cargo", "pointer", "1.0.0", "MIT OR Apache-2.0", g.spdx_dnf("MIT OR Apache-2.0"),
              [("package:LICENSE.md", APACHE_POINTER)], "rust:t")
        row = self.only(b)  # pdfium-render case: canonical Apache text plus the pointer
        self.assertEqual(row["selected"], "Apache-2.0")
        self.assertEqual({t["origin"] for t in row["texts"]}, {"package:LICENSE.md", "canonical:Apache-2.0:fixture"})
        b = self.builder()
        b.add("cargo", "m", "1.0.0", "MIT", [("MIT",)], [("package:NOTICE", MIT)], "rust:t")
        self.assertEqual(b.unresolved[-1]["reason"], "license_text_missing")
        b = self.builder()
        b.add("cargo", "u", "1.0.0", "MIT", [("MIT",)], [("package:LICENSE", b"unrecognised wording\n")], "rust:t")
        self.assertEqual(b.unresolved[-1]["reason"], "license_text_missing")

    def test_reviewed_package_file_counts_only_when_hash_bound(self):
        body = b"unrecognised MIT wording\n"
        b = self.builder()
        b.add("cargo", "u", "1.0.0", "MIT", [("MIT",)], [("package:LICENSE", body)], "rust:t",
              reviewed=[{"file": "LICENSE", "sha256": sha(body), "license": "MIT"}])
        self.assertIn("reviewed: package:LICENSE accepted as MIT text", self.only(b)["warnings"])
        with self.assertRaisesRegex(g.Refused, "^reviewed_file_binding$"):
            self.builder().add("cargo", "u", "1.0.0", "MIT", [("MIT",)], [("package:LICENSE", body)], "rust:t",
                               reviewed=[{"file": "LICENSE", "sha256": "0" * 64, "license": "MIT"}])

    def test_unselected_texts_drop_but_kept_and_notice_files_ship(self):
        b = self.builder()
        b.add("cargo", "z-sys", "1.0.0", "BSD-3-Clause", g.spdx_dnf("BSD-3-Clause"),
              [("package:LICENSE", BSD, False), ("package:COPYING", GPL, False),
               ("package:zstd/COPYING", GPL, True), ("package:NOTICE", b"Attribution only\n", False)], "rust:t")
        origins = [t["origin"] for t in self.only(b)["texts"]]
        self.assertEqual(origins, ["package:LICENSE", "package:NOTICE", "package:zstd/COPYING"])

    def test_canonical_text_only_for_holder_independent_license(self):
        b = self.builder()
        b.add("maven", "g:a", "1", "Apache-2.0", [("Apache-2.0",)], [], ":desktop:runtimeClasspath")
        row = self.only(b)
        self.assertEqual(row["texts"][0]["origin"], "canonical:Apache-2.0:fixture")
        self.assertTrue(row["warnings"])

    def test_unknown_and_forbidden_licenses_fail(self):
        b = self.builder()
        b.add("cargo", "gpl", "1.0.0", "GPL-3.0-only", g.spdx_dnf("GPL-3.0-only"), [("package:COPYING", GPL)], "rust:t")
        b.add("cargo", "odd", "1.0.0", "WTFPL", g.spdx_dnf("WTFPL"), [("package:LICENSE", b"x\n")], "rust:t")
        self.assertEqual([r["reason"] for r in b.unresolved], ["forbidden_license", "unknown_license"])
        self.assertEqual(b.components, {})
        b.add("cargo", "dual", "1.0.0", "GPL-2.0 OR MIT", g.spdx_dnf("GPL-2.0 OR MIT"),
              [("package:COPYING", GPL), ("package:LICENSE-MIT", MIT)], "rust:t")
        row = self.only(b)
        self.assertEqual(row["selected"], "MIT")
        self.assertEqual([t["origin"] for t in row["texts"]], ["package:LICENSE-MIT"])

    def test_declaration_only_ships_exactly_the_named_files(self):
        files = [("package:vendor/COPYING", BSD, True), ("package:vendor/PATENTS", b"Additional patent grant\n", True)]
        declaration = {"files": [{"file": "vendor/COPYING", "sha256": sha(BSD)},
                                 {"file": "vendor/PATENTS", "sha256": sha(b"Additional patent grant\n")}]}
        b = self.builder()
        b.add("cargo", "w-sys", "1.0.0", "MIT", [("MIT",)], files, "rust:t")
        self.assertEqual(b.unresolved[-1]["reason"], "license_text_missing")
        b = self.builder()
        b.add("cargo", "w-sys", "1.0.0", "MIT", [("MIT",)], files, "rust:t", declaration=declaration)
        row = self.only(b)
        self.assertEqual(row["selected"], "MIT")
        self.assertEqual([t["origin"] for t in row["texts"]], ["package:vendor/COPYING", "package:vendor/PATENTS"])
        self.assertTrue(row["warnings"][0].startswith("declaration only:"))
        with self.assertRaisesRegex(g.Refused, "^declaration_file_binding$"):
            self.builder().add("cargo", "w-sys", "1.0.0", "MIT", [("MIT",)], files[:1], "rust:t",
                               declaration=declaration)

    def test_forced_selection_must_be_a_declared_branch(self):
        b = self.builder()
        b.add("maven", "jna", "5", "LGPL-2.1-or-later; Apache-2.0", [("LGPL-2.1-or-later",), ("Apache-2.0",)],
              [("package:x.jar!META-INF/LICENSE", b"dual license summary\n")], ":desktop:runtimeClasspath",
              extra=[("retained:third_party/notices/AL2.txt", APACHE)], forced="Apache-2.0")
        self.assertEqual(self.only(b)["selected"], "Apache-2.0")
        with self.assertRaisesRegex(g.Refused, "^supplement_selection$"):
            self.builder().add("maven", "jna", "5", "x", [("LGPL-2.1-or-later", "Apache-2.0")], [],
                               ":desktop:runtimeClasspath", forced="Apache-2.0")  # AND is not OR
        with self.assertRaisesRegex(g.Refused, "^supplement_selection$"):
            self.builder().add("cargo", "c", "1.0.0", "MIT", [("MIT",)], [], "rust:t", forced="Apache-2.0")


class Fixture(unittest.TestCase):
    """A complete synthetic repository plus caches and toolchains for generate()."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vw-notices-unit-")
        self.base = Path(self.temp.name)
        self.root = self.base / "repo"
        self.cargo_home = self.base / "cargo"
        self.gradle_cache = self.base / "gradle"
        self.report = self.base / "report"
        self.retained = {}
        self.write_root()
        self.write_cargo()
        self.write_gradle()
        self.write_toolchains()

    def tearDown(self):
        self.temp.cleanup()

    def put(self, path: Path, body: bytes) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
        return path

    def retain(self, name: str, body: bytes, license_id: str, covers=None) -> str:
        self.put(self.root / "third_party/notices" / name, body)
        self.retained[name] = {"path": "notices/" + name, "license": license_id,
                               "source_url": "https://example.test/" + name, "sha256": sha(body)}
        if covers:
            self.retained[name]["covers"] = covers
        self.put(self.root / "third_party/LICENSES", json.dumps(
            {"schema_version": 1, "files": sorted(self.retained.values(), key=lambda r: r["path"])}).encode())
        return "third_party/notices/" + name

    def supplements(self, components):
        path = self.root / "tools/licenses/notice-supplements.json"
        value = json.loads(path.read_bytes())
        value["components"] = components
        path.write_bytes(json.dumps(value).encode())

    def write_root(self):
        apache = self.retain("Apache-2.0.txt", APACHE, "Apache-2.0")
        node = self.retain("Node-1.0.0-LICENSE.txt", MIT, "MIT")
        npm_notice = self.retain("npm-left-pad-1.0.0-LICENSE.txt", MIT, "MIT")
        self.put(self.root / "tools/licenses/notice-supplements.json", json.dumps({
            "schema": 1,
            "canonical_texts": {"Apache-2.0": {"path": apache, "sha256": sha(APACHE)}},
            "components": [],
            "bundled": [{"id": "node-runtime", "record": "tools/licenses/node-runtime.json", "name": "Node",
                         "license": "MIT", "notice": {"path": node}}],
        }).encode())
        self.put(self.root / "tools/licenses/node-runtime.json", json.dumps({
            "version": "1.0.0", "signature_verified": True, "node_sha256": "1" * 64, "license_sha256": sha(MIT)}).encode())
        self.put(self.root / "tools/licenses/reviewed-maven.json", b'{"schema": 1, "artifacts": []}')
        # npm: one production and one dev package under the existing admission policy.
        package = self.put(self.root / "mcp/package.json", b'{"name": "fixture"}').read_bytes()
        lock = {"lockfileVersion": 3, "packages": {"": {"name": "fixture"}}}
        rows = []
        for name, dev in (("left-pad", False), ("dev-tool", True)):
            entry = {"version": "1.0.0", "resolved": f"https://registry.npmjs.org/{name}/-/{name}-1.0.0.tgz",
                     "integrity": "sha512-" + "A" * 86 + "==", "license": "MIT"}
            if dev:
                entry["dev"] = True
            lock["packages"]["node_modules/" + name] = entry
            rows.append({**{k: entry[k] for k in ("version", "resolved", "integrity", "license")},
                         "path": "node_modules/" + name, "dev": dev, "package_sha256": "2" * 64,
                         "notices": [{"name": "LICENSE", "retained": npm_notice.removeprefix("third_party/"),
                                      "sha256": sha(MIT)}]})
        lock_raw = self.put(self.root / "mcp/package-lock.json", json.dumps(lock).encode()).read_bytes()
        self.put(self.root / "tools/licenses/reviewed-npm.json", json.dumps(
            {"schema": 1, "lock_sha256": sha(lock_raw), "package_sha256": sha(package), "packages": rows}).encode())

    def write_cargo(self):
        index = self.cargo_home / "registry/cache/index.fixture"
        archives = {
            ("shipped", "1.0.0"): crate("shipped-1.0.0", {"LICENSE-MIT": MIT, "LICENSE-APACHE": APACHE,
                                                           "src/lib.rs": b"//"}),
            ("native-sys", "0.1.0"): crate("native-sys-0.1.0", {"LICENSE": MIT, "vendor/lib/LICENSE": BSD,
                                                                 "vendor/tests/LICENSE": GPL}),
            ("devonly", "1.0.0"): crate("devonly-1.0.0", {"LICENSE": MIT}),
            ("buildonly", "1.0.0"): crate("buildonly-1.0.0", {"LICENSE": MIT}),
            ("derive", "1.0.0"): crate("derive-1.0.0", {"LICENSE": MIT}),
        }
        lock = ["version = 4", ""]
        packages, nodes = [], []

        def pid(name, version):
            return f"registry+https://github.com/rust-lang/crates.io-index#{name}@{version}"

        for (name, version), body in archives.items():
            self.put(index / f"{name}-{version}.crate", body)
            lock += ["[[package]]", f'name = "{name}"', f'version = "{version}"',
                     'source = "registry+https://github.com/rust-lang/crates.io-index"', f'checksum = "{sha(body)}"', ""]
            kind = ["proc-macro"] if name == "derive" else ["lib"]
            packages.append({"id": pid(name, version), "name": name, "version": version, "source": g.CRATES_IO,
                             "license": "MIT OR Apache-2.0" if name == "shipped" else "MIT",
                             "links": "native" if name == "native-sys" else None, "targets": [{"kind": kind}]})
            nodes.append({"id": pid(name, version), "deps": []})
        normal, dev, build = [{"kind": None, "target": None}], [{"kind": "dev", "target": None}], [{"kind": "build", "target": None}]
        roots = {}
        for triple, names in g.RUST_ROOTS.items():
            for name in names:
                roots[name] = f"path+file:///fixture/{name}#0.1.0"
        members = sorted(set(roots.values()))
        for name, ident in sorted(roots.items()):
            dependencies = ["shipped", "native-sys", "devonly", "buildonly", "derive"] if name == "vw-ffi" else []
            lock += ["[[package]]", f'name = "{name}"', 'version = "0.1.0"',
                     "dependencies = [" + ", ".join(f'"{d}"' for d in dependencies) + "]", ""]
            packages.append({"id": ident, "name": name, "version": "0.1.0", "source": None,
                             "license": "LicenseRef-VisualWorkbench-Proprietary", "targets": [{"kind": ["lib"]}],
                             "features": {"default": [], "extra": []}})
            deps = []
            if name == "vw-ffi":
                deps = [{"pkg": pid("shipped", "1.0.0"), "dep_kinds": normal},
                        {"pkg": pid("native-sys", "0.1.0"), "dep_kinds": normal},
                        {"pkg": pid("devonly", "1.0.0"), "dep_kinds": dev},
                        {"pkg": pid("buildonly", "1.0.0"), "dep_kinds": build},
                        {"pkg": pid("derive", "1.0.0"), "dep_kinds": normal}]
            nodes.append({"id": ident, "deps": deps, "features": ["default", "extra"]})
        self.put(self.root / "Cargo.lock", "\n".join(lock).encode())
        self.metadata_values = {triple: json.loads(json.dumps({"version": 1, "packages": packages,
                                                               "resolve": {"nodes": nodes}, "workspace_members": members}))
                                for triple in g.RUST_ROOTS}

    @property
    def metadata(self):
        return {triple: json.dumps(value).encode() for triple, value in self.metadata_values.items()}

    def pom(self, group, artifact, version, licenses=("The Apache Software License, Version 2.0",), packaging=None):
        rows = "".join(f"<license><name>{name}</name></license>" for name in licenses)
        extra = f"<packaging>{packaging}</packaging>" if packaging else ""
        return (f'<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>'
                f"<groupId>{group}</groupId><artifactId>{artifact}</artifactId><version>{version}</version>{extra}"
                f"<licenses>{rows}</licenses></project>").encode()

    def cache_file(self, group, artifact, version, name, body, directory=None):
        directory = directory or hashlib.sha1(body).hexdigest()
        return self.put(self.gradle_cache / group / artifact / version / directory / name, body)

    def replace_jar(self, artifact, version, body):
        for old in (self.gradle_cache / "org.example" / artifact / version).glob(f"*/{artifact}-{version}.jar"):
            old.unlink()
        self.cache_file("org.example", artifact, version, f"{artifact}-{version}.jar", body)

    def write_gradle(self):
        gradle = sys.modules.get("check_gradle_licenses") or g._load(
            "check_gradle_licenses", g.REPOSITORY / "tools/check_gradle_licenses.py")
        configs = sorted(gradle.EXPECTED_CONFIGURATIONS)
        modules = {
            "org.example:with-license:1.0": ([":android:debugRuntimeClasspath"], ("The MIT License",), "jar"),
            "org.example:apache-bare:2.0": ([":desktop:runtimeClasspath"], ("Apache License, Version 2.0",), "jar"),
            "org.example:kmp-root:3.0": ([":desktop:runtimeClasspath"], ("Apache-2.0",), "redirect"),
            "org.example:kmp-root-jvm:3.0": ([":desktop:runtimeClasspath"], ("Apache-2.0",), "jar"),
            "org.example:bom:4.0": ([":desktop:runtimeClasspath"], ("Apache-2.0",), "pom"),
            "org.example:bench-only:5.0": ([":video-bench:debugRuntimeClasspath"], ("Apache-2.0",), "jar"),
            "org.example:dual:6.0": ([":android:debugRuntimeClasspath"], ("Apache-2.0",), "module"),
        }
        census = {name: [] for name in configs}
        rows = []
        (self.report / "poms").mkdir(parents=True)
        for key, (scopes, licenses, kind) in modules.items():
            group, artifact, version = key.split(":")
            body = self.pom(group, artifact, version, licenses, "pom" if kind == "pom" else None)
            relative = f"poms/{sha(key.encode())}.pom"
            self.put(self.report / relative, body)
            rows.append({"moduleName": f"{group}:{artifact}", "moduleVersion": version, "pomPath": relative,
                         "pomSha256": sha(body), "configurations": sorted(scopes), "parentPomChain": []})
            for scope in scopes:
                census[scope].append(key)
            self.cache_file(group, artifact, version, f"{artifact}-{version}.pom", body)
            if kind == "jar":
                files = {"com/x/A.class": b"\xca\xfe"}
                if artifact == "with-license":
                    files["META-INF/LICENSE"] = MIT
                self.cache_file(group, artifact, version, f"{artifact}-{version}.jar", jar(files))
            elif kind == "redirect":
                module = {"variants": [{"name": "jvmRuntimeElements-published",
                                        "attributes": {"org.gradle.usage": "java-runtime", "org.gradle.category": "library"},
                                        "available-at": {"group": group, "module": artifact + "-jvm", "version": version}},
                                       {"name": "metadataApiElements", "attributes": {"org.gradle.usage": "kotlin-metadata"},
                                        "files": [{"name": "x-metadata.jar", "sha256": "0" * 64}]}]}
                self.cache_file(group, artifact, version, f"{artifact}-{version}.module", json.dumps(module).encode())
            elif kind == "module":
                # Android and standard-JVM runtime variants; only the Android file
                # is cached, as for a graph that only selects the Android variant.
                aar = jar({"classes.jar": jar({"com/x/A.class": b"\xca\xfe"})})
                self.cache_file(group, artifact, version, "dual.aar", aar)
                module = {"variants": [
                    {"name": "androidRuntime", "attributes": {"org.gradle.usage": "java-runtime",
                                                              "org.gradle.jvm.environment": "android"},
                     "files": [{"name": "dual.aar", "sha256": sha(aar)}]},
                    {"name": "jvmRuntime", "attributes": {"org.gradle.usage": "java-runtime",
                                                          "org.gradle.jvm.environment": "standard-jvm"},
                     "files": [{"name": "dual-jre.jar", "sha256": "1" * 64}]}]}
                self.cache_file(group, artifact, version, f"{artifact}-{version}.module", json.dumps(module).encode())
        for name in configs:  # the existing reader requires every configuration to be nonempty
            if not census[name]:
                census[name].append("org.example:with-license:1.0" if name in g.GRADLE_SHIPPING
                                    else "org.example:bench-only:5.0")
        for row in rows:
            key = row["moduleName"] + ":" + row["moduleVersion"]
            row["configurations"] = sorted(n for n, keys in census.items() if key in keys)
        self.put(self.report / "dependencies.json", json.dumps({
            "format": "application-pom-census-v1", "configurations": {k: sorted(v) for k, v in census.items()},
            "dependencies": rows, "parentPoms": []}).encode())

    def write_toolchains(self):
        self.rust = self.base / "rustup/toolchains/1.99.0-x86_64-pc-windows-msvc"
        self.put(self.rust / "lib/rustlib/multirust-channel-manifest.toml",
                 b'manifest-version = "2"\n\n[pkg.rust]\nversion = "1.99.0 (fixture 2026-10-01)"\n')
        for triple in g.RUST_ROOTS:
            self.put(self.rust / f"lib/rustlib/manifest-rust-std-{triple}", b"file:lib/rustlib/x\n")
        self.put(self.rust / "share/doc/rust/COPYRIGHT-library.html", b"<html>std notices " + MIT + b"</html>\n")
        self.put(self.rust / "share/doc/rust/licenses/MIT.txt", MIT)
        self.put(self.rust / "share/doc/rust/licenses/Apache-2.0.txt", APACHE)
        self.ndk = self.base / "sdk/ndk" / g.NDK_VERSION
        self.put(self.ndk / "source.properties", f"Pkg.Desc = Android NDK\nPkg.Revision = {g.NDK_VERSION}\n".encode())
        self.put(self.ndk / "toolchains/llvm/prebuilt/windows-x86_64/NOTICE",
                 b"The LLVM Project is under the Apache License v2.0 with LLVM Exceptions:\n" + APACHE)

    def generate(self, **overrides):
        options = dict(metadata=self.metadata, cargo_home=self.cargo_home, gradle_report=self.report / "dependencies.json",
                       gradle_cache=self.gradle_cache, npm_modules=None, desktop_image=None,
                       rust_toolchain=self.rust, android_ndk=self.ndk)
        options.update(overrides)
        return g.generate(self.root, **options)


class Generation(Fixture):
    def test_shipped_scope_texts_and_exclusions(self):
        text, raw, receipt = self.generate()
        self.assertEqual(receipt["status"], "complete", receipt["unresolved"])
        self.assertEqual(receipt["skipped"], [])
        names = {(c["ecosystem"], c["name"]) for c in receipt["components"]}
        self.assertIn(("cargo", "shipped"), names)
        self.assertIn(("cargo", "native-sys"), names)
        for excluded in ("devonly", "buildonly", "derive"):
            self.assertNotIn(("cargo", excluded), names)
        self.assertIn("derive 1.0.0", receipt["excluded"]["cargo:proc-macro-build-host-only"])
        self.assertIn("devonly 1.0.0", receipt["excluded"]["cargo:not-in-shipped-graph(dev/build/other-target/tool)"])
        self.assertIn("buildonly 1.0.0", receipt["excluded"]["cargo:not-in-shipped-graph(dev/build/other-target/tool)"])
        self.assertIn("vw-ffi", receipt["excluded"]["cargo:first-party-workspace"])
        native = next(c for c in receipt["components"] if c["name"] == "native-sys")
        # Vendored notices below a native crate's root always ship; tests/ is skipped.
        self.assertEqual([t["origin"] for t in native["texts"]], ["package:LICENSE", "package:vendor/lib/LICENSE"])
        shipped = next(c for c in receipt["components"] if c["name"] == "shipped")
        self.assertEqual(shipped["selected"], "MIT")
        self.assertEqual(shipped["scope"], ["rust:aarch64-linux-android", "rust:x86_64-pc-windows-msvc"])
        self.assertEqual(shipped["sources"]["crate_sha256"],
                         sha((self.cargo_home / "registry/cache/index.fixture/shipped-1.0.0.crate").read_bytes()))
        for triple, value in self.metadata.items():
            entry = receipt["inputs"][f"cargo-metadata/{triple}.json"]
            self.assertEqual((entry["bytes"], entry["sha256"]), (len(value), sha(value)))
        # Maven: product configurations only; redirect and BOM have no own bytes.
        self.assertIn(("maven", "org.example:with-license"), names)
        self.assertIn(("maven", "org.example:kmp-root-jvm"), names)
        self.assertIn(("maven", "org.example:dual"), names)  # the uncached JRE variant is not selected
        self.assertNotIn(("maven", "org.example:kmp-root"), names)
        self.assertNotIn(("maven", "org.example:bom"), names)
        self.assertNotIn(("maven", "org.example:bench-only"), names)
        self.assertIn("org.example:bench-only:5.0", receipt["excluded"]["maven:non-product-configuration"])
        bare = next(c for c in receipt["components"] if c["name"] == "org.example:apache-bare")
        self.assertTrue(bare["texts"][0]["origin"].startswith("canonical:Apache-2.0:"))
        licensed = next(c for c in receipt["components"] if c["name"] == "org.example:with-license")
        self.assertEqual(licensed["texts"][0]["origin"], "package:with-license-1.0.jar!META-INF/LICENSE")
        # npm production only; bundled runtime and toolchains from their records.
        self.assertIn(("npm", "left-pad"), names)
        self.assertNotIn(("npm", "dev-tool"), names)
        for bundled in ("node-runtime", g.RUST_STD, g.NDK_RUNTIME):
            self.assertIn(("bundled", bundled), names)
        std = next(c for c in receipt["components"] if c["name"] == g.RUST_STD)
        self.assertEqual(len(std["texts"]), 3)
        self.assertNotIn("bundled:java-runtime", receipt["coverage"])
        self.assertIn(MIT, text)
        self.assertEqual(receipt["output"]["sha256"], sha(text))
        summary = g.verify_bundle(raw, text, artifact_kinds=frozenset({"opaque", "apk"}), opaque_coverage=frozenset())
        self.assertEqual(summary["output_sha256"], sha(text))

    def test_output_is_deterministic(self):
        first = self.generate()
        second = self.generate()
        self.assertEqual(first[0], second[0])
        self.assertEqual(first[1], second[1])
        self.assertNotIn(str(self.base).encode(), first[1])  # no host paths in the receipt
        self.assertNotIn(str(self.base).encode(), first[0])

    def test_crate_archive_must_match_cargo_lock(self):
        self.put(self.cargo_home / "registry/cache/index.fixture/shipped-1.0.0.crate",
                 crate("shipped-1.0.0", {"LICENSE-MIT": MIT + b"tampered"}))
        _, _, receipt = self.generate()
        self.assertEqual(receipt["status"], "incomplete")
        self.assertIn({"component": "cargo:shipped@1.0.0", "reason": "crate_source_unavailable",
                       "detail": "registry cache archive matching Cargo.lock checksum not found"}, receipt["unresolved"])

    def test_metadata_flags_are_checked_from_content(self):
        _, _, receipt = self.generate()
        entry = receipt["inputs"]["cargo-metadata/x86_64-pc-windows-msvc.json"]
        self.assertEqual(entry["origin"], "supplied")
        self.assertIn("--all-features --filter-platform x86_64-pc-windows-msvc", entry["required_command"])
        nodes = self.metadata_values["x86_64-pc-windows-msvc"]["resolve"]["nodes"]
        ffi = next(node for node in nodes if "vw-ffi" in node["id"])
        ffi["features"] = ["default"]  # not --all-features
        with self.assertRaisesRegex(g.Refused, "^cargo_metadata_flags$"):
            self.generate()
        ffi["features"] = ["default", "extra"]
        ffi["deps"][0]["dep_kinds"] = [{"kind": None, "target": "cfg(unix)"}]  # not --filter-platform windows
        with self.assertRaisesRegex(g.Refused, "^cargo_metadata_flags$"):
            self.generate()
        ffi["deps"][0]["dep_kinds"] = [{"kind": None, "target": "cfg(windows)"}]
        self.assertEqual(self.generate()[2]["status"], "complete")
        # Cargo keeps an applicable edge with all of its kinds, including false ones.
        ffi["deps"][0]["dep_kinds"] = [{"kind": None, "target": "cfg(unix)"}, {"kind": None, "target": "cfg(windows)"}]
        self.assertEqual(self.generate()[2]["status"], "complete")
        del self.metadata_values["aarch64-linux-android"]
        with self.assertRaisesRegex(g.Refused, "^rust_target$"):
            self.generate()

    def test_cfg_evaluation_for_product_targets(self):
        for expression, windows, android in (
                ("cfg(windows)", True, False), ("cfg(unix)", False, True),
                ('cfg(target_os = "android")', False, True), ('cfg(all(unix, not(target_os = "macos")))', False, True),
                ('cfg(any(target_os = "linux", target_os = "android"))', False, True),
                ('cfg(target_feature = "sse2")', None, None), ('cfg(all(windows, target_feature = "x"))', None, False),
                ("x86_64-pc-windows-msvc", True, False)):
            with self.subTest(expression=expression):
                self.assertIs(g.cfg_value(expression, "x86_64-pc-windows-msvc"), windows)
                self.assertIs(g.cfg_value(expression, "aarch64-linux-android"), android)

    def test_metadata_must_agree_with_cargo_lock(self):
        for value in self.metadata_values.values():
            for package in value["packages"]:
                if package["name"] == "shipped":
                    package["version"] = "9.9.9"
        with self.assertRaisesRegex(g.Refused, "^cargo_metadata_lock_mismatch$"):
            self.generate()

    def test_metadata_edges_must_exist_in_cargo_lock(self):
        lock = (self.root / "Cargo.lock").read_text()
        self.put(self.root / "Cargo.lock", lock.replace('"shipped", ', "").encode())
        with self.assertRaisesRegex(g.Refused, "^cargo_metadata_lock_mismatch$"):
            self.generate()

    def test_forbidden_crate_license_makes_bundle_incomplete_and_unassemblable(self):
        for package in (p for value in self.metadata_values.values() for p in value["packages"]):
            if package["name"] == "shipped":
                package["license"] = "AGPL-3.0-only"
        text, raw, receipt = self.generate()
        self.assertIn("forbidden_license", {r["reason"] for r in receipt["unresolved"]})
        with self.assertRaisesRegex(g.Refused, "^notices_incomplete$"):
            g.verify_bundle(raw, text)

    def test_cargo_supplement_selection_is_validated(self):
        self.supplements([{"ecosystem": "cargo", "name": "shipped", "version": "1.0.0", "selected": "BSD-3-Clause",
                           "notices": [], "reason": "fixture"}])
        with self.assertRaisesRegex(g.Refused, "^supplement_selection$"):
            self.generate()

    def test_supplements_are_parsed_strictly(self):
        path = self.root / "tools/licenses/notice-supplements.json"
        raw = path.read_bytes()
        path.write_bytes(raw.replace(b'"schema": 1,', b'"schema": 1, "schema": 1,'))
        with self.assertRaisesRegex(g.Refused, "^supplement_schema$"):
            self.generate()
        path.write_bytes(raw)
        for row, code in (({"ecosystem": "cargo", "name": "x", "version": "1", "notices": [], "reason": "r",
                            "unexpected": 1}, "supplement_row"),
                          ({"ecosystem": "maven", "name": "x", "version": "1", "notices": [], "reason": "r",
                            "native_review": "free text"}, "supplement_native_review"),
                          ({"ecosystem": "cargo", "name": "x", "version": "1", "notices": [], "reason": "r",
                            "declaration_only": True}, "supplement_declaration")):
            with self.subTest(code=code):
                self.supplements([row])
                with self.assertRaisesRegex(g.Refused, "^" + code + "$"):
                    self.generate()

    def test_reviewed_supplements_resolve_texts_and_native_gate(self):
        for package in (p for value in self.metadata_values.values() for p in value["packages"]):
            if package["name"] == "native-sys":
                package["license"] = "MPL-2.0"  # ships only MIT/BSD files: text missing
        self.replace_jar("apache-bare", "2.0", jar({"lib/x.dll": b"MZ", "com/x/A.class": b"\xca\xfe"}))
        _, _, receipt = self.generate()
        self.assertEqual({r["reason"] for r in receipt["unresolved"]},
                         {"license_text_missing", "native_notice_unestablished"})
        mpl = self.retain("MPL-2.0.txt", MPL, "MPL-2.0")
        self.supplements([
            {"ecosystem": "cargo", "name": "native-sys", "version": "0.1.0", "selected": None,
             "notices": [{"path": mpl, "sha256": sha(MPL)}], "reason": "fixture",
             "notes": ["MPL-2.0 Source Code Form: https://example.test/native-sys"]},
            {"ecosystem": "maven", "name": "org.example:apache-bare", "version": "2.0", "selected": None,
             "notices": [], "reason": "fixture",
             "native_review": {"source_url": "https://example.test/src", "revision": "abc123",
                               "summary": "x.dll is the project's own Apache-2.0 JNI code", "reviewed": "2026-11-04"}}])
        text, raw, receipt = self.generate()
        self.assertEqual(receipt["status"], "complete", receipt["unresolved"])
        native = next(c for c in receipt["components"] if c["name"] == "native-sys")
        self.assertIn("retained:" + mpl, [t["origin"] for t in native["texts"]])
        self.assertIn(b"note: MPL-2.0 Source Code Form: https://example.test/native-sys", text)
        bare = next(c for c in receipt["components"] if c["name"] == "org.example:apache-bare")
        self.assertIn("native review: x.dll is the project's own Apache-2.0 JNI code (source https://example.test/src "
                      "at abc123, reviewed 2026-11-04)", bare["warnings"])
        self.assertEqual(g.verify_bundle(raw, text)["status"], "complete")

    def test_embedded_native_components_need_a_covering_notice(self):
        self.replace_jar("apache-bare", "2.0", jar({
            "lib/x.dll": b"MZ...deflate 1.3 Copyright Jean-loup Gailly and Mark Adler...wuffs_base...VP8X..VP8L..WEBP",
            "LICENSE": APACHE}))
        _, _, receipt = self.generate()
        self.assertIn({"component": "maven:org.example:apache-bare:2.0", "reason": "embedded_native_notice_missing",
                       "detail": "bundled native code contains Wuffs, libwebp, zlib with no notice in the shipped texts"},
                      receipt["unresolved"])
        # zlib is covered by its text's content; Wuffs ships a generic Apache-2.0
        # text and is covered by an explicit `covers` registration. A build
        # configuration can add libraries the probe cannot see.
        zlib = self.retain("zlib-LICENSE.txt", b"zlib License\nThis software is provided 'as-is'\n", "Zlib")
        wuffs = self.retain("wuffs-LICENSE.txt", APACHE, "Apache-2.0", covers=["Wuffs"])
        webp = self.retain("libwebp-COPYING.txt", BSD + b"libwebp\n", "BSD-3-Clause")
        row = {"ecosystem": "maven", "name": "org.example:apache-bare", "version": "2.0", "selected": None,
               "reason": "fixture", "notices": [{"path": item, "sha256": sha(self.root.joinpath(item).read_bytes())}
                                                for item in (zlib, wuffs, webp)],
               "build_components": {"components": ["D3D12MemoryAllocator"], "evidence": ["https://example.test/gn"]}}
        self.supplements([row])
        _, _, receipt = self.generate()
        self.assertIn({"component": "maven:org.example:apache-bare:2.0", "reason": "embedded_native_notice_missing",
                       "detail": "bundled native code contains D3D12MemoryAllocator with no notice in the shipped texts"},
                      receipt["unresolved"])
        d3d = self.retain("D3D12MA-LICENSE.txt", MIT + b"D3D12 Memory Allocator\n", "MIT")
        row["notices"].append({"path": d3d, "sha256": sha(self.root.joinpath(d3d).read_bytes())})
        self.supplements([row])
        _, _, receipt = self.generate()
        self.assertEqual(receipt["status"], "complete", receipt["unresolved"])
        bare = next(c for c in receipt["components"] if c["name"] == "org.example:apache-bare")
        self.assertEqual(bare["sources"]["embedded_native_components"],
                         ["D3D12MemoryAllocator", "Wuffs", "libwebp", "zlib"])

    def test_covers_registration_requires_matching_license_text(self):
        bogus = self.retain("wuffs-LICENSE.txt", b"not an Apache text\n", "Apache-2.0", covers=["Wuffs"])
        self.supplements([{"ecosystem": "maven", "name": "org.example:apache-bare", "version": "2.0", "selected": None,
                           "reason": "fixture", "notices": [{"path": bogus, "sha256": sha(b"not an Apache text\n")}]}])
        with self.assertRaisesRegex(g.Refused, "^retained_covers_family$"):
            self.generate()

    def test_gradle_artifacts_fail_closed(self):
        for path in (self.gradle_cache / "org.example/with-license/1.0").glob("*/with-license-1.0.jar"):
            path.unlink()
        _, _, receipt = self.generate()
        self.assertIn({"component": "maven:org.example:with-license:1.0", "reason": "gradle_artifact_unavailable",
                       "detail": "not in the local Gradle cache: artifact"}, receipt["unresolved"])

    def test_partial_runtime_variant_files_fail_closed(self):
        module = next((self.gradle_cache / "org.example/dual/6.0").glob("*/dual-6.0.module"))
        value = json.loads(module.read_bytes())
        value["variants"][0]["files"].append({"name": "dual-extra.jar", "sha256": "2" * 64})
        module.write_bytes(json.dumps(value).encode())
        _, _, receipt = self.generate()
        self.assertIn({"component": "maven:org.example:dual:6.0", "reason": "gradle_artifact_partial",
                       "detail": "not in the local Gradle cache: dual-extra.jar"}, receipt["unresolved"])

    def test_gradle_cache_candidates_are_hash_bound(self):
        good = next((self.gradle_cache / "org.example/with-license/1.0").glob("*/with-license-1.0.jar"))
        # A stale candidate in another directory is skipped when a valid one exists.
        self.cache_file("org.example", "with-license", "1.0", "with-license-1.0.jar", b"stale", directory="0" * 40)
        _, _, receipt = self.generate()
        self.assertEqual(receipt["status"], "complete", receipt["unresolved"])
        good.write_bytes(good.read_bytes() + b"tampered")
        with self.assertRaisesRegex(g.Refused, "^gradle_cache_hash$"):
            self.generate()

    def test_missing_gradle_report_refuses_unless_skipped(self):
        with self.assertRaisesRegex(g.Refused, "^gradle_report_missing$"):
            self.generate(gradle_report=self.base / "absent.json")
        text, raw, receipt = self.generate(gradle_report=None, skip=frozenset({"gradle"}))
        self.assertNotIn(":desktop:runtimeClasspath", receipt["coverage"])
        self.assertEqual(receipt["skipped"], ["gradle"])
        with self.assertRaisesRegex(g.Refused, "^notices_undeclared_skip$"):
            g.verify_bundle(raw, text)
        self.assertEqual(g.verify_bundle(raw, text, declared_skips=frozenset({"gradle"}))["skipped"], ["gradle"])

    def test_toolchains_must_be_the_pinned_versions(self):
        self.put(self.rust / "lib/rustlib/multirust-channel-manifest.toml", b'[pkg.rust]\nversion = "1.98.0 (x)"\n')
        with self.assertRaisesRegex(g.Refused, "^rust_toolchain_version$"):
            self.generate()
        self.write_toolchains()
        self.put(self.ndk / "source.properties", b"Pkg.Revision = 29.0.1\n")
        with self.assertRaisesRegex(g.Refused, "^android_ndk_version$"):
            self.generate()

    def test_java_runtime_legal_files_from_desktop_image(self):
        image = self.base / "image"
        self.put(image / "runtime/release", b'JAVA_VERSION="21.0.9"\nIMPLEMENTOR="Fixture"\n')
        self.put(image / "runtime/legal/java.base/LICENSE", GPL + b"CLASSPATH EXCEPTION\n")
        self.put(image / "runtime/legal/java.desktop/zlib.md", b"zlib notice\n")
        text, raw, receipt = self.generate(desktop_image=image)
        java = next(c for c in receipt["components"] if c["name"] == g.JAVA_RUNTIME)
        self.assertEqual(java["selected"], g.JAVA_EXCEPTION)
        self.assertEqual(java["version"], "21.0.9")
        self.assertEqual(len(java["texts"]), 2)
        self.assertTrue(g.DESKTOP_COVERAGE <= set(receipt["coverage"]))
        kinds = frozenset({"desktop-file", "apk", "external-package"})
        self.assertEqual(g.verify_bundle(raw, text, artifact_kinds=kinds)["status"], "complete")
        delivered = {"runtime/legal/java.base/LICENSE": sha(GPL + b"CLASSPATH EXCEPTION\n"),
                     "runtime/legal/java.desktop/zlib.md": sha(b"zlib notice\n"),
                     "app/resources/node.exe": "1" * 64, "app/x.jar": "3" * 64}
        self.assertEqual(g.verify_bundle(raw, text, artifact_kinds=kinds, desktop_files=delivered)["status"], "complete")
        with self.assertRaisesRegex(g.Refused, "^notices_java_runtime_mismatch$"):
            g.verify_bundle(raw, text, artifact_kinds=kinds,
                            desktop_files=delivered | {"runtime/legal/java.base/ADDITIONAL": "4" * 64})
        with self.assertRaisesRegex(g.Refused, "^notices_node_mismatch$"):
            g.verify_bundle(raw, text, artifact_kinds=kinds, desktop_files=delivered | {"app/resources/node.exe": "5" * 64})


class Verification(Fixture):
    def test_tampering_and_policy_are_detected(self):
        text, raw, receipt = self.generate()
        with self.assertRaisesRegex(g.Refused, "^notices_text_binding$"):
            g.verify_bundle(raw, text.replace(b"VISUAL WORKBENCH", b"VISUAL-WORKBENCH"))
        forged = json.loads(raw)
        forged["components"][0]["selected"] = "GPL-3.0-only"
        texts = {}
        for digest in forged["texts"]:
            match = re.search(rb"=== TEXT " + digest.encode() + rb" (\d+) ===\n", text)
            start = match.end()
            texts[digest] = text[start:start + int(match.group(1))]
        forged.pop("output")
        new_text, new_raw, _ = g.finalize(forged, texts)
        with self.assertRaisesRegex(g.Refused, "^notices_forbidden_license$"):
            g.verify_bundle(new_raw, new_text)
        for kinds in ({"desktop-file"}, {"external-package"}):
            with self.assertRaisesRegex(g.Refused, "^notices_desktop_coverage$"):
                g.verify_bundle(raw, text, artifact_kinds=frozenset(kinds))  # no Java runtime here
        with self.assertRaisesRegex(g.Refused, "^notices_opaque_coverage_undeclared$"):
            g.verify_bundle(raw, text, artifact_kinds=frozenset({"opaque"}))
        with self.assertRaisesRegex(g.Refused, "^notices_opaque_coverage$"):
            g.verify_bundle(raw, text, artifact_kinds=frozenset({"opaque"}),
                            opaque_coverage=frozenset({"bundled:" + g.JAVA_RUNTIME}))  # not generated here
        self.assertEqual(g.verify_bundle(raw, text, artifact_kinds=frozenset({"opaque"}),
                                         opaque_coverage=frozenset({"rust:x86_64-pc-windows-msvc"}))["status"], "complete")
        with self.assertRaisesRegex(g.Refused, "^notices_stale_input$"):
            g.verify_bundle(raw, text, declarations={"Cargo.lock": {"bytes": 1, "sha256": "0" * 64}})
        self.assertEqual(g.verify_bundle(raw, text, declarations={"Cargo.lock": receipt["inputs"]["Cargo.lock"]})
                         ["status"], "complete")


class RepositoryPolicy(unittest.TestCase):
    def test_supplements_are_bound_to_retained_notices(self):
        supplements = g.load_supplements(g.REPOSITORY)
        self.assertEqual(set(g.canonical_texts(g.REPOSITORY, supplements)), {"Apache-2.0"})
        for row in supplements["components"]:
            g.supplement_entries(g.REPOSITORY, row)
        record = json.loads((g.REPOSITORY / "tools/licenses/node-runtime.json").read_bytes())
        for row in supplements["bundled"]:
            g.retained(g.REPOSITORY, row["notice"]["path"], record["license_sha256"])

    def test_policy_allow_list_is_the_gate_allow_list(self):
        self.assertEqual(g.ALLOWED, frozenset(g.SETUP.ALLOWED))
        self.assertNotIn(g.JAVA_EXCEPTION, g.ALLOWED)


if __name__ == "__main__":
    unittest.main()
