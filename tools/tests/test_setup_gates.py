"""Policy regressions: reject unknown/copy-left dependencies and unbound binaries."""
from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tempfile
import unittest
from copy import deepcopy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from check_gradle_licenses import check_report, read_pom_report
from check_setup import check_third_party


def module(name: str, *licenses: str) -> dict:
    return {"moduleName": name, "moduleVersion": "1.0", "moduleLicenses": [{"spdxLicense": value} for value in licenses]}


class GradleLicenseTests(unittest.TestCase):
    def test_permissive_dependency_passes(self):
        self.assertEqual(check_report({"dependencies": [module("sample:library", "Apache-2.0")]}), (1, []))

    def test_gpl_dependency_fails(self):
        self.assertTrue(check_report({"dependencies": [module("sample:gpl", "GPL-3.0-only")]})[1])

    def test_unknown_dependency_fails(self):
        self.assertTrue(check_report({"dependencies": [module("sample:unknown", "custom terms")]})[1])

    def test_missing_dependency_license_fails(self):
        self.assertTrue(check_report({"dependencies": [module("sample:missing")]})[1])

    def test_empty_report_fails(self):
        self.assertTrue(check_report({"dependencies": []})[1])

    def test_jna_explicit_apache_option_passes(self):
        self.assertFalse(check_report({"dependencies": [module("net.java.dev.jna:jna", "LGPL-2.1-or-later", "Apache-2.0")]})[1])

    def test_unapproved_dual_license_does_not_silently_pass(self):
        self.assertTrue(check_report({"dependencies": [module("sample:ambiguous", "Apache-2.0", "GPL-3.0-only")]})[1])


class PomCensusTests(unittest.TestCase):
    """Require complete runtime coverage and licenses bound to the resolved POM bytes."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="vw-pom-gate-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = pathlib.Path(self.temporary.name)
        self.key = "sample:library:1.0"
        self.relative_path = "poms/" + hashlib.sha256(self.key.encode()).hexdigest() + ".pom"
        self.pom_file = self.directory / self.relative_path
        self.pom_file.parent.mkdir()
        self.configurations = {
            ":android:debugRuntimeClasspath": [self.key],
            ":android:releaseRuntimeClasspath": [self.key],
            ":desktop:runtimeClasspath": [self.key],
            ":shared:desktopRuntimeClasspath": [self.key],
            ":pen-probe:debugRuntimeClasspath": [self.key],
            ":pen-probe:releaseRuntimeClasspath": [self.key],
            ":video-bench:debugRuntimeClasspath": [self.key],
            ":video-bench:releaseRuntimeClasspath": [self.key],
        }
        self.report = {
            "format": "application-pom-census-v1",
            "gradleVersion": "9.7.0",
            "configurations": deepcopy(self.configurations),
            "dependencies": [{
                "moduleName": "sample:library",
                "moduleVersion": "1.0",
                "configurations": sorted(self.configurations),
                "pomPath": self.relative_path,
            }],
        }
        self.pom = (
            '<project xmlns="http://maven.apache.org/POM/4.0.0">'
            '<modelVersion>4.0.0</modelVersion><groupId>sample</groupId>'
            '<artifactId>library</artifactId><version>1.0</version>'
            '<licenses><license><name>Apache-2.0</name>'
            '<url>https://www.apache.org/licenses/LICENSE-2.0</url>'
            '</license></licenses></project>'
        ).encode()
        self.write_pom(self.pom)

    def write_pom(self, contents: bytes):
        self.pom_file.write_bytes(contents)
        self.report["dependencies"][0]["pomSha256"] = hashlib.sha256(contents).hexdigest()

    def make_pom(self, key: str, *, licenses=(), parent: str | None = None) -> bytes:
        group, artifact, version = key.split(":")
        parent_xml = ""
        if parent is not None:
            parent_group, parent_artifact, parent_version = parent.split(":")
            parent_xml = (
                f"<parent><groupId>{parent_group}</groupId>"
                f"<artifactId>{parent_artifact}</artifactId>"
                f"<version>{parent_version}</version></parent>"
            )
        license_xml = ""
        if licenses:
            license_xml = "<licenses>" + "".join(
                f"<license><name>{name}</name></license>" for name in licenses
            ) + "</licenses>"
        return (
            '<project xmlns="http://maven.apache.org/POM/4.0.0">'
            f"<modelVersion>4.0.0</modelVersion>{parent_xml}"
            f"<groupId>{group}</groupId><artifactId>{artifact}</artifactId>"
            f"<version>{version}</version>{license_xml}</project>"
        ).encode()

    def add_parent_pom(self, key: str, *, licenses=(), parent: str | None = None) -> dict:
        name, version = key.rsplit(":", 1)
        relative_path = "poms/" + hashlib.sha256(key.encode()).hexdigest() + ".pom"
        contents = self.make_pom(key, licenses=licenses, parent=parent)
        (self.directory / relative_path).write_bytes(contents)
        record = {
            "moduleName": name,
            "moduleVersion": version,
            "pomPath": relative_path,
            "pomSha256": hashlib.sha256(contents).hexdigest(),
        }
        self.report.setdefault("parentPoms", []).append(record)
        return record

    def bind_parent_chain(self, *parents: str, licenses=()):
        self.write_pom(self.make_pom(self.key, licenses=licenses, parent=parents[0]))
        self.report["dependencies"][0]["parentPomChain"] = list(parents)

    def test_complete_bound_runtime_census_passes(self):
        self.assertEqual(check_report(read_pom_report(self.report, self.directory)), (1, []))

    def test_every_required_runtime_configuration_is_mandatory(self):
        for configuration in self.configurations:
            with self.subTest(configuration=configuration):
                report = deepcopy(self.report)
                report["configurations"].pop(configuration)
                with self.assertRaises(ValueError):
                    read_pom_report(report, self.directory)

    def test_empty_runtime_configuration_fails(self):
        report = deepcopy(self.report)
        report["configurations"][":android:releaseRuntimeClasspath"] = []
        with self.assertRaises(ValueError):
            read_pom_report(report, self.directory)

    def test_missing_dependency_metadata_fails(self):
        report = deepcopy(self.report)
        report["configurations"][":desktop:runtimeClasspath"].append("sample:missing:1.0")
        with self.assertRaises(ValueError):
            read_pom_report(report, self.directory)

    def test_duplicate_module_metadata_fails(self):
        report = deepcopy(self.report)
        report["dependencies"].append(deepcopy(report["dependencies"][0]))
        with self.assertRaises(ValueError):
            read_pom_report(report, self.directory)

    def test_incomplete_module_configuration_binding_fails(self):
        report = deepcopy(self.report)
        report["dependencies"][0]["configurations"].remove(":android:releaseRuntimeClasspath")
        with self.assertRaises(ValueError):
            read_pom_report(report, self.directory)

    def test_pom_hash_mismatch_fails(self):
        self.pom_file.write_bytes(self.pom.replace(b"Apache-2.0</name>", b"GPL-3.0-only</name>"))
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_unsafe_or_unbound_pom_path_fails(self):
        for path in ("../outside.pom", "poms/another-module.pom"):
            with self.subTest(path=path):
                report = deepcopy(self.report)
                report["dependencies"][0]["pomPath"] = path
                with self.assertRaises(ValueError):
                    read_pom_report(report, self.directory)

    def test_missing_pom_file_fails(self):
        self.pom_file.unlink()
        with self.assertRaises(OSError):
            read_pom_report(self.report, self.directory)

    def test_hash_bound_pom_for_another_module_fails(self):
        self.write_pom(self.pom.replace(b"<artifactId>library</artifactId>", b"<artifactId>other</artifactId>"))
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_invalid_xml_namespace_and_doctype_fail(self):
        for contents in (
            self.pom[:-1],
            self.pom.replace(b"http://maven.apache.org/POM/4.0.0", b"https://example.invalid/other"),
            b'<!DOCTYPE project [<!ENTITY license "Apache-2.0">]>' + self.pom,
        ):
            with self.subTest(contents=contents[:60]):
                self.write_pom(contents)
                with self.assertRaises(ValueError):
                    read_pom_report(self.report, self.directory)

    def test_permissive_url_does_not_override_license_declaration(self):
        for label in ("GPL-3.0-only", "custom terms", "The BSD License"):
            with self.subTest(label=label):
                self.write_pom(self.pom.replace(b"Apache-2.0</name>", label.encode() + b"</name>"))
                count, issues = check_report(read_pom_report(self.report, self.directory))
                self.assertEqual(count, 1)
                self.assertTrue(issues)

    def test_parent_reference_does_not_supply_unretrieved_license(self):
        contents = self.pom.replace(
            b"<licenses>",
            b"<parent><groupId>sample</groupId><artifactId>parent</artifactId><version>1.0</version></parent><licenses>",
        )
        license_start, license_end = contents.index(b"<licenses>"), contents.index(b"</licenses>") + len(b"</licenses>")
        self.write_pom(contents[:license_start] + contents[license_end:])
        count, issues = check_report(read_pom_report(self.report, self.directory))
        self.assertEqual(count, 1)
        self.assertTrue(issues)

    def test_one_level_bound_parent_license_passes(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        annotated = read_pom_report(self.report, self.directory)
        self.assertEqual(check_report(annotated), (1, []))
        self.assertEqual(annotated["dependencies"][0]["moduleLicenses"][0]["moduleLicense"], "Apache-2.0")
        self.assertEqual(annotated["dependencies"][0]["licenseSourcePom"], parent)

    def test_two_level_bound_parent_license_passes(self):
        parent, ancestor = "sample:parent:1.0", "sample:ancestor:2.0"
        self.add_parent_pom(parent, parent=ancestor)
        self.add_parent_pom(ancestor, licenses=("MIT",))
        self.bind_parent_chain(parent, ancestor)
        annotated = read_pom_report(self.report, self.directory)
        self.assertEqual(check_report(annotated), (1, []))
        self.assertEqual(annotated["dependencies"][0]["moduleLicenses"][0]["moduleLicense"], "MIT")
        self.assertEqual(annotated["dependencies"][0]["licenseSourcePom"], ancestor)

    def test_direct_child_license_takes_precedence_without_parent_chain(self):
        parent = "sample:parent:1.0"
        self.write_pom(self.make_pom(self.key, licenses=("Apache-2.0",), parent=parent))
        self.assertEqual(check_report(read_pom_report(self.report, self.directory)), (1, []))

    def test_direct_child_license_rejects_parent_chain(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, licenses=("MIT",))
        self.bind_parent_chain(parent, licenses=("Apache-2.0",))
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_inherited_gpl_license_fails(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, licenses=("GPL-3.0-only",))
        self.bind_parent_chain(parent)
        count, issues = check_report(read_pom_report(self.report, self.directory))
        self.assertEqual(count, 1)
        self.assertTrue(issues)

    def test_parent_pom_hash_mismatch_fails(self):
        parent = "sample:parent:1.0"
        record = self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        (self.directory / record["pomPath"]).write_bytes(self.make_pom(parent, licenses=("GPL-3.0-only",)))
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_missing_parent_pom_file_fails(self):
        parent = "sample:parent:1.0"
        record = self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        (self.directory / record["pomPath"]).unlink()
        with self.assertRaises(OSError):
            read_pom_report(self.report, self.directory)

    def test_missing_parent_metadata_fails(self):
        self.bind_parent_chain("sample:parent:1.0")
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_parent_chain_must_match_direct_parent_coordinates(self):
        declared, substituted = "sample:parent:1.0", "sample:other:1.0"
        self.add_parent_pom(substituted, licenses=("Apache-2.0",))
        self.bind_parent_chain(declared)
        self.report["dependencies"][0]["parentPomChain"] = [substituted]
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_parent_declaration_and_fields_must_use_pom_namespace(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        original = self.make_pom(self.key, parent=parent)
        for contents in (
            original.replace(b"<parent>", b'<parent xmlns="https://example.invalid/other">'),
            original.replace(
                b"<artifactId>parent</artifactId>",
                b'<artifactId xmlns="https://example.invalid/other">parent</artifactId>',
            ),
        ):
            with self.subTest(contents=contents):
                self.write_pom(contents)
                with self.assertRaises(ValueError):
                    read_pom_report(self.report, self.directory)

    def test_parent_coordinates_cannot_be_property_interpolations(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        original = self.make_pom(self.key, parent=parent)
        for tag, value, expression in (
            ("groupId", "sample", "${project.groupId}"),
            ("artifactId", "parent", "${parent.artifactId}"),
            ("version", "1.0", "${revision}"),
        ):
            with self.subTest(tag=tag):
                self.write_pom(original.replace(
                    f"<{tag}>{value}</{tag}>".encode(),
                    f"<{tag}>{expression}</{tag}>".encode(),
                    1,
                ))
                with self.assertRaises(ValueError):
                    read_pom_report(self.report, self.directory)

    def test_hash_bound_parent_pom_for_another_module_fails(self):
        parent = "sample:parent:1.0"
        record = self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        contents = self.make_pom("sample:other:1.0", licenses=("Apache-2.0",))
        (self.directory / record["pomPath"]).write_bytes(contents)
        record["pomSha256"] = hashlib.sha256(contents).hexdigest()
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_parent_pom_path_must_be_bound_to_parent_coordinates(self):
        parent = "sample:parent:1.0"
        record = self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        for path in ("../outside.pom", self.relative_path):
            with self.subTest(path=path):
                report = deepcopy(self.report)
                report["parentPoms"][0]["pomPath"] = path
                with self.assertRaises(ValueError):
                    read_pom_report(report, self.directory)

    def test_unused_parent_metadata_fails(self):
        self.add_parent_pom("sample:unused:1.0", licenses=("Apache-2.0",))
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_duplicate_parent_metadata_fails(self):
        parent = "sample:parent:1.0"
        record = self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        self.report["parentPoms"].append(deepcopy(record))
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_duplicate_parent_in_chain_fails(self):
        parent, ancestor = "sample:parent:1.0", "sample:ancestor:2.0"
        self.add_parent_pom(parent, parent=ancestor)
        self.add_parent_pom(ancestor, parent=parent)
        self.bind_parent_chain(parent, ancestor, parent)
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_parent_chain_cycle_back_to_dependency_fails(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, parent=self.key)
        self.bind_parent_chain(parent, self.key)
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_parent_chain_depth_greater_than_eight_fails(self):
        parents = [f"sample:parent-{index}:1.0" for index in range(9)]
        for index, parent in enumerate(parents):
            if index + 1 < len(parents):
                self.add_parent_pom(parent, parent=parents[index + 1])
            else:
                self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(*parents)
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_parent_chain_depth_exactly_eight_passes(self):
        parents = [f"sample:parent-{index}:1.0" for index in range(8)]
        for index, parent in enumerate(parents):
            if index + 1 < len(parents):
                self.add_parent_pom(parent, parent=parents[index + 1])
            else:
                self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(*parents)
        annotated = read_pom_report(self.report, self.directory)
        self.assertEqual(check_report(annotated), (1, []))
        self.assertEqual(annotated["dependencies"][0]["licenseSourcePom"], parents[-1])

    def test_trailing_chain_after_nearest_licensed_parent_fails(self):
        parent, ancestor = "sample:parent:1.0", "sample:ancestor:2.0"
        self.add_parent_pom(parent, licenses=("Apache-2.0",), parent=ancestor)
        self.add_parent_pom(ancestor, licenses=("GPL-3.0-only",))
        self.bind_parent_chain(parent, ancestor)
        with self.assertRaises(ValueError):
            read_pom_report(self.report, self.directory)

    def test_invalid_parent_chain_shape_fails(self):
        parent = "sample:parent:1.0"
        self.add_parent_pom(parent, licenses=("Apache-2.0",))
        self.bind_parent_chain(parent)
        for chain in (parent, {"parent": parent}, [None], ["sample:parent"]):
            with self.subTest(chain=chain):
                report = deepcopy(self.report)
                report["dependencies"][0]["parentPomChain"] = chain
                with self.assertRaises(ValueError):
                    read_pom_report(report, self.directory)


class BundledBinaryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="vw-gate-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name)
        self.third_party = self.root / "third_party"
        self.third_party.mkdir()
        self.binary = self.third_party / "fixture.bin"
        self.binary.write_bytes(b"synthetic test fixture")

    def write_manifest(self, **overrides):
        entry = {"path": "fixture.bin", "license": "MIT", "source_url": "https://example.invalid/fixture", "sha256": hashlib.sha256(self.binary.read_bytes()).hexdigest()}
        entry.update(overrides)
        (self.third_party / "LICENSES").write_text(json.dumps({"schema_version": 1, "files": [entry]}), encoding="utf-8")

    def test_bound_binary_passes(self):
        self.write_manifest()
        self.assertEqual(check_third_party(self.root), [])

    def test_binary_hash_mismatch_fails(self):
        self.write_manifest(sha256="0" * 64)
        self.assertTrue(check_third_party(self.root))

    def test_gpl_binary_fails(self):
        self.write_manifest(license="GPL-3.0-only")
        self.assertTrue(check_third_party(self.root))

    def test_path_traversal_fails(self):
        self.write_manifest(path="../outside.bin")
        self.assertTrue(check_third_party(self.root))

    def test_unlisted_file_fails(self):
        self.write_manifest()
        (self.third_party / "unlisted.bin").write_bytes(b"another synthetic fixture")
        self.assertTrue(check_third_party(self.root))


if __name__ == "__main__":
    unittest.main()
