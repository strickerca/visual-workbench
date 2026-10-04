"""Exact supplemental-license evidence cannot approve changed or missing bytes."""
import hashlib
import json
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import check_gradle_licenses as gate


class ReviewedMavenTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="vw-reviewed-maven-")
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)
        self.directory = self.root / "report"
        self.key = "fixture:verifier:0.2.0"
        self.artifact = self.directory / ("artifacts/" + hashlib.sha256(self.key.encode()).hexdigest() + ".aar")
        self.artifact.parent.mkdir(parents=True)
        self.artifact.write_bytes(b"synthetic-artifact")
        self.notice = self.root / "third_party/notices/fixture.txt"
        self.notice.parent.mkdir(parents=True)
        self.notice.write_bytes(b"synthetic-notice")
        self.policy_path = self.root / "tools/licenses/reviewed-maven.json"
        self.policy_path.parent.mkdir(parents=True)
        self.row = {
            "coordinate": self.key, "extension": "aar", "pom_sha256": "a" * 64,
            "bytes": len(b"synthetic-artifact"), "sha256": hashlib.sha256(self.artifact.read_bytes()).hexdigest(),
            "selected_license": "MIT", "license_source_url": "https://example.invalid/pinned-notice",
            "notices": [{"path": "third_party/notices/fixture.txt", "sha256": hashlib.sha256(self.notice.read_bytes()).hexdigest()}],
        }
        self.policy = {"schema": 1, "artifacts": [self.row]}
        self.module = {"pomSha256": "a" * 64, "reviewedArtifactPath": self.artifact.relative_to(self.directory).as_posix()}
        self.write_policy()
        override = patch.object(gate, "REPOSITORY", self.root)
        override.start(); self.addCleanup(override.stop)

    def write_policy(self):
        self.policy_path.write_text(json.dumps(self.policy), encoding="utf-8")

    def read(self):
        return gate.reviewed_artifact_licenses(self.key, self.module, self.directory)

    def test_exact_bound_record_supplies_only_selected_spdx(self):
        self.assertEqual(self.read(), [{"spdxLicense": "MIT", "moduleLicenseUrl": self.row["license_source_url"]}])

    def test_unknown_coordinate_never_inherits_approval(self):
        self.assertEqual(gate.reviewed_artifact_licenses("fixture:verifier:0.2.1", self.module, self.directory), [])

    def test_changed_pom_refuses(self):
        self.module["pomSha256"] = "b" * 64
        with self.assertRaises(ValueError): self.read()

    def test_changed_artifact_size_or_same_size_bytes_refuse(self):
        for contents in (b"short", b"x" * self.row["bytes"]):
            self.artifact.write_bytes(contents)
            with self.assertRaises(ValueError): self.read()

    def test_missing_or_unbound_artifact_refuses(self):
        self.module.pop("reviewedArtifactPath")
        with self.assertRaises(ValueError): self.read()
        self.module["reviewedArtifactPath"] = "../other.aar"
        with self.assertRaises(ValueError): self.read()

    def test_changed_or_escaped_notice_refuses(self):
        self.notice.write_bytes(b"changed")
        with self.assertRaises(ValueError): self.read()
        self.row["notices"][0] = {"path": "report/" + self.module["reviewedArtifactPath"], "sha256": self.row["sha256"]}
        self.write_policy()
        with self.assertRaises(ValueError): self.read()

    def test_duplicate_policy_coordinates_refuse(self):
        self.policy["artifacts"].append(self.row.copy()); self.write_policy()
        with self.assertRaises(ValueError): self.read()

    def test_supplemental_source_does_not_bypass_license_allowlist(self):
        self.row["selected_license"] = "GPL-3.0-only"; self.write_policy()
        self.assertTrue(gate.check_report({"dependencies": [{
            "moduleName": "fixture:verifier", "moduleVersion": "0.2.0", "moduleLicenses": self.read()
        }]})[1])


if __name__ == "__main__": unittest.main()
