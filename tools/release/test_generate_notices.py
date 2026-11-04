"""Small synthetic, stdlib-only checks for LIC-003 notice generation.

No cargo, Gradle, npm, network or real package caches are used; every graph,
archive and license file below is a fixture built in a temporary directory.
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
BSD = b"Copyright (c) Fixture\nRedistribution and use in source and binary forms, with or without modification,\n" \
      b"* Neither the name of the copyright holder nor the names of its contributors\n"
GPL = b"                    GNU GENERAL PUBLIC LICENSE\n                       Version 2, June 1991\n"


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
        self.assertEqual(g.detect(BSD), {"bsd"})
        self.assertEqual(g.detect(GPL), {"gpl"})
        self.assertEqual(g.detect(b"Dual licensed, see the other files"), frozenset())
        for name in ("LICENSE", "LICENSE-MIT", "COPYING", "license.md", "NOTICE.txt", "UNLICENSE", "PATENTS",
                     "META-INF/LICENSE.txt", "MIT-LICENSE", "THIRD_PARTY_NOTICE.txt"):
            self.assertTrue(g.is_license_file(name), name)
        for name in ("license.rs", "README.md", "LicenseCheck.class", "src/licenses.json", "x.kotlin_module"):
            self.assertFalse(g.is_license_file(name), name)


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

    def test_unselected_license_texts_are_not_shipped_but_notices_are(self):
        b = self.builder()
        b.add("cargo", "z-sys", "1.0.0", "BSD-3-Clause", g.spdx_dnf("BSD-3-Clause"),
              [("package:LICENSE", BSD), ("package:zstd/COPYING", GPL), ("package:NOTICE", b"Attribution only\n")],
              "rust:t")
        origins = [t["origin"] for t in self.only(b)["texts"]]
        self.assertEqual(origins, ["package:LICENSE", "package:NOTICE"])

    def test_canonical_text_only_for_holder_independent_license(self):
        b = self.builder()
        b.add("maven", "g:a", "1", "Apache-2.0", [("Apache-2.0",)], [], ":desktop:runtimeClasspath")
        row = self.only(b)
        self.assertEqual(row["texts"][0]["origin"], "canonical:Apache-2.0:fixture")
        self.assertTrue(row["warnings"])
        b.add("cargo", "m", "1.0.0", "MIT", [("MIT",)], [("package:NOTICE", b"just a notice\n")], "rust:t")
        self.assertEqual(b.unresolved[-1]["reason"], "license_text_missing")

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

    def test_forced_selection_and_retained_texts(self):
        b = self.builder()
        b.add("maven", "jna", "5", "LGPL-2.1-or-later; Apache-2.0", [("LGPL-2.1-or-later", "Apache-2.0")],
              [("package:x.jar!META-INF/LICENSE", b"dual license summary\n")], ":desktop:runtimeClasspath",
              extra=[("retained:third_party/notices/libffi.txt", MIT)], forced="Apache-2.0")
        row = self.only(b)
        self.assertEqual(row["selected"], "Apache-2.0")
        self.assertIn("retained:third_party/notices/libffi.txt", [t["origin"] for t in row["texts"]])


class Fixture(unittest.TestCase):
    """A complete synthetic repository plus caches for generate()."""

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

    def tearDown(self):
        self.temp.cleanup()

    def put(self, path: Path, body: bytes) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
        return path

    def retain(self, name: str, body: bytes, license_id: str) -> str:
        self.put(self.root / "third_party/notices" / name, body)
        self.retained[name] = {"path": "notices/" + name, "license": license_id,
                               "source_url": "https://example.test/" + name, "sha256": sha(body)}
        return "third_party/notices/" + name

    def write_root(self):
        apache = self.retain("Apache-2.0.txt", APACHE, "Apache-2.0")
        node = self.retain("Node-1.0.0-LICENSE.txt", MIT, "MIT")
        npm_notice = self.retain("npm-left-pad-1.0.0-LICENSE.txt", MIT, "MIT")
        self.put(self.root / "third_party/LICENSES", json.dumps(
            {"schema_version": 1, "files": sorted(self.retained.values(), key=lambda r: r["path"])}).encode())
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
        self.put(self.root / "Cargo.lock", "\n".join(lock).encode())
        normal, dev, build = [{"kind": None, "target": None}], [{"kind": "dev", "target": None}], [{"kind": "build", "target": None}]
        roots = {}
        for triple, names in g.RUST_ROOTS.items():
            for name in names:
                roots[name] = f"path+file:///fixture/{name}#0.1.0"
        members = sorted(set(roots.values()))
        for name, ident in sorted(roots.items()):
            packages.append({"id": ident, "name": name, "version": "0.1.0", "source": None,
                             "license": "LicenseRef-VisualWorkbench-Proprietary", "targets": [{"kind": ["lib"]}]})
            deps = []
            if name == "vw-ffi":
                deps = [{"pkg": pid("shipped", "1.0.0"), "dep_kinds": normal},
                        {"pkg": pid("native-sys", "0.1.0"), "dep_kinds": normal},
                        {"pkg": pid("devonly", "1.0.0"), "dep_kinds": dev},
                        {"pkg": pid("buildonly", "1.0.0"), "dep_kinds": build},
                        {"pkg": pid("derive", "1.0.0"), "dep_kinds": normal}]
            nodes.append({"id": ident, "deps": deps})
        self.metadata = {triple: {"packages": packages, "resolve": {"nodes": nodes}, "workspace_members": members}
                         for triple in g.RUST_ROOTS}

    def pom(self, group, artifact, version, licenses=("The Apache Software License, Version 2.0",), packaging=None):
        rows = "".join(f"<license><name>{name}</name></license>" for name in licenses)
        extra = f"<packaging>{packaging}</packaging>" if packaging else ""
        return (f'<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>'
                f"<groupId>{group}</groupId><artifactId>{artifact}</artifactId><version>{version}</version>{extra}"
                f"<licenses>{rows}</licenses></project>").encode()

    def cache_file(self, group, artifact, version, name, body, sha1_dir=True):
        directory = hashlib.sha1(body).hexdigest() if sha1_dir else "f" * 40
        return self.put(self.gradle_cache / group / artifact / version / directory / name, body)

    def write_gradle(self):
        gradle = sys.modules.get("check_gradle_licenses") or g._load(
            "check_gradle_licenses", g.REPOSITORY / "tools/check_gradle_licenses.py")
        configs = sorted(gradle.EXPECTED_CONFIGURATIONS)
        modules = {
            "org.example:with-license:1.0": (configs[:1] or [":android:debugRuntimeClasspath"],
                                              ("The MIT License",), "jar"),
            "org.example:apache-bare:2.0": ([":desktop:runtimeClasspath"], ("Apache License, Version 2.0",), "jar"),
            "org.example:kmp-root:3.0": ([":desktop:runtimeClasspath"], ("Apache-2.0",), "redirect"),
            "org.example:kmp-root-jvm:3.0": ([":desktop:runtimeClasspath"], ("Apache-2.0",), "jar"),
            "org.example:bom:4.0": ([":desktop:runtimeClasspath"], ("Apache-2.0",), "pom"),
            "org.example:bench-only:5.0": ([":video-bench:debugRuntimeClasspath"], ("Apache-2.0",), "jar"),
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

    def generate(self, **overrides):
        options = dict(metadata=self.metadata, cargo_home=self.cargo_home, gradle_report=self.report / "dependencies.json",
                       gradle_cache=self.gradle_cache, npm_modules=None, desktop_image=None)
        options.update(overrides)
        return g.generate(self.root, **options)


class Generation(Fixture):
    def test_shipped_scope_texts_and_exclusions(self):
        text, raw, receipt = self.generate()
        self.assertEqual(receipt["status"], "complete", receipt["unresolved"])
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
        self.assertEqual([t["origin"] for t in native["texts"]], ["package:LICENSE"])  # vendor BSD not MIT-selected
        shipped = next(c for c in receipt["components"] if c["name"] == "shipped")
        self.assertEqual(shipped["selected"], "MIT")
        self.assertEqual(shipped["scope"], ["rust:aarch64-linux-android", "rust:x86_64-pc-windows-msvc"])
        self.assertEqual(shipped["sources"]["crate_sha256"],
                         sha((self.cargo_home / "registry/cache/index.fixture/shipped-1.0.0.crate").read_bytes()))
        # Maven: product configurations only; redirect and BOM have no own bytes.
        self.assertIn(("maven", "org.example:with-license"), names)
        self.assertIn(("maven", "org.example:kmp-root-jvm"), names)
        self.assertNotIn(("maven", "org.example:kmp-root"), names)
        self.assertNotIn(("maven", "org.example:bom"), names)
        self.assertNotIn(("maven", "org.example:bench-only"), names)
        self.assertIn("org.example:bench-only:5.0", receipt["excluded"]["maven:non-product-configuration"])
        bare = next(c for c in receipt["components"] if c["name"] == "org.example:apache-bare")
        self.assertTrue(bare["texts"][0]["origin"].startswith("canonical:Apache-2.0:"))
        licensed = next(c for c in receipt["components"] if c["name"] == "org.example:with-license")
        self.assertEqual(licensed["texts"][0]["origin"], "package:with-license-1.0.jar!META-INF/LICENSE")
        # npm production only; bundled runtime from its record.
        self.assertIn(("npm", "left-pad"), names)
        self.assertNotIn(("npm", "dev-tool"), names)
        self.assertIn(("bundled", "node-runtime"), names)
        self.assertNotIn("bundled:java-runtime", receipt["coverage"])
        self.assertIn(MIT, text)
        self.assertEqual(receipt["output"]["sha256"], sha(text))
        summary = g.verify_bundle(raw, text, artifact_kinds=frozenset({"opaque"}))
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

    def test_forbidden_crate_license_makes_bundle_incomplete_and_unassemblable(self):
        for package in self.metadata["x86_64-pc-windows-msvc"]["packages"]:
            if package["name"] == "shipped":
                package["license"] = "AGPL-3.0-only"
        text, raw, receipt = self.generate()
        self.assertIn("forbidden_license", {r["reason"] for r in receipt["unresolved"]})
        with self.assertRaisesRegex(g.Refused, "^notices_incomplete$"):
            g.verify_bundle(raw, text)

    def test_native_library_without_notice_is_unresolved(self):
        body = jar({"lib/x.dll": b"MZ", "com/x/A.class": b"\xca\xfe"})
        self.cache_file("org.example", "apache-bare", "2.0", "apache-bare-2.0.jar", body)
        for old in (self.gradle_cache / "org.example/apache-bare/2.0").glob("*/apache-bare-2.0.jar"):
            if old.read_bytes() != body:
                old.unlink()
        _, _, receipt = self.generate()
        self.assertIn({"component": "maven:org.example:apache-bare:2.0", "reason": "native_notice_unestablished",
                       "detail": "1 bundled native libraries and no package license file"}, receipt["unresolved"])

    def test_gradle_cache_bytes_are_hash_bound(self):
        for path in (self.gradle_cache / "org.example/with-license/1.0").glob("*/with-license-1.0.jar"):
            path.write_bytes(path.read_bytes() + b"tampered")
        with self.assertRaisesRegex(g.Refused, "^gradle_cache_hash$"):
            self.generate()

    def test_missing_gradle_report_refuses_unless_skipped(self):
        with self.assertRaisesRegex(g.Refused, "^gradle_report_missing$"):
            self.generate(gradle_report=self.base / "absent.json")
        _, _, receipt = self.generate(gradle_report=None, skip=frozenset({"gradle"}))
        self.assertNotIn(":desktop:runtimeClasspath", receipt["coverage"])

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
        self.assertEqual(g.verify_bundle(raw, text, artifact_kinds=frozenset({"desktop-file", "apk"}))["status"],
                         "complete")


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
        with self.assertRaisesRegex(g.Refused, "^notices_desktop_coverage$"):
            g.verify_bundle(raw, text, artifact_kinds=frozenset({"desktop-file"}))
        with self.assertRaisesRegex(g.Refused, "^notices_stale_input$"):
            g.verify_bundle(raw, text, declarations={"Cargo.lock": {"bytes": 1, "sha256": "0" * 64}})
        self.assertEqual(g.verify_bundle(raw, text, declarations={"Cargo.lock": receipt["inputs"]["Cargo.lock"]})
                         ["status"], "complete")


class RepositoryPolicy(unittest.TestCase):
    def test_supplements_are_bound_to_retained_notices(self):
        supplements = g.load_supplements(g.REPOSITORY)
        self.assertEqual(set(g.canonical_texts(g.REPOSITORY, supplements)), {"Apache-2.0"})
        for row in supplements["components"]:
            for notice in row["notices"]:
                g.retained(g.REPOSITORY, notice["path"], notice["sha256"])
        record = json.loads((g.REPOSITORY / "tools/licenses/node-runtime.json").read_bytes())
        for row in supplements["bundled"]:
            g.retained(g.REPOSITORY, row["notice"]["path"], record["license_sha256"])

    def test_policy_allow_list_is_the_gate_allow_list(self):
        self.assertEqual(g.ALLOWED, frozenset(g.SETUP.ALLOWED))
        self.assertNotIn(g.JAVA_EXCEPTION, g.ALLOWED)


if __name__ == "__main__":
    unittest.main()
