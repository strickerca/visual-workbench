"""Exact lock/notice and installed graph refusal regressions."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_npm_licenses import verify


class NpmLicenseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.install = self.root / "mcp"
        sha = lambda data: hashlib.sha256(data).hexdigest()
        self.meta = b'{"name":"sample","version":"1.0.0","license":"MIT"}'
        self.row = {"path": "node_modules/sample", "version": "1.0.0", "license": "MIT", "dev": False,
                    "resolved": "https://registry.npmjs.org/sample/-/sample-1.0.0.tgz",
                    "integrity": "sha512-" + "a" * 86 + "==", "package_sha256": sha(self.meta),
                    "notices": [{"name": "LICENSE", "retained": "notices/sample.txt", "sha256": sha(b"MIT notice")} ]}
        self.put("mcp/package.json", b'{"private":true}')
        self.lock = {"lockfileVersion": 3, "packages": {"": {}, self.row["path"]: {
            k: self.row[k] for k in ("version", "license", "integrity", "resolved")}}}
        self.put("mcp/package-lock.json", json.dumps(self.lock).encode())
        self.policy = {"schema": 1, "lock_sha256": sha((self.install / "package-lock.json").read_bytes()),
                       "package_sha256": sha((self.install / "package.json").read_bytes()), "packages": [self.row]}
        self.save_policy()
        self.put("third_party/notices/sample.txt", b"MIT notice")
        self.put("mcp/node_modules/sample/package.json", self.meta)
        self.put("mcp/node_modules/sample/LICENSE", b"MIT notice")

    def put(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def save_policy(self):
        self.put("tools/licenses/reviewed-npm.json", json.dumps(self.policy).encode())

    def test_exact_graph_and_notices_are_admitted(self):
        self.assertEqual(verify(self.root, installed=self.install)["admitted_packages"], 1)

    def test_changed_lock_cannot_self_approve(self):
        self.put("mcp/package-lock.json", b"{}")
        with self.assertRaisesRegex(ValueError, "unreviewed_lock"):
            verify(self.root)

    def test_unchanged_license_label_does_not_hide_modified_notice(self):
        self.put("mcp/node_modules/sample/LICENSE", b"changed terms")
        with self.assertRaisesRegex(ValueError, "installed_notice"):
            verify(self.root, installed=self.install)

    def test_retained_notice_must_match_reviewed_bytes(self):
        self.put("third_party/notices/sample.txt", b"changed")
        with self.assertRaisesRegex(ValueError, "notice_binding"):
            verify(self.root)

    def test_unreviewed_extra_package_is_refused(self):
        self.put("mcp/node_modules/foreign/package.json", b"{}")
        with self.assertRaisesRegex(ValueError, "installed_graph"):
            verify(self.root, installed=self.install)

    def test_installed_metadata_drift_is_refused(self):
        self.put("mcp/node_modules/sample/package.json", self.meta + b" ")
        with self.assertRaisesRegex(ValueError, "installed_metadata"):
            verify(self.root, installed=self.install)

    def test_notice_path_cannot_escape_policy_root(self):
        self.row["notices"][0]["retained"] = "../outside"
        self.save_policy()
        with self.assertRaisesRegex(ValueError, "path"):
            verify(self.root)

    def test_nonallowed_policy_license_is_refused(self):
        self.row["license"] = "unknown"
        self.save_policy()
        with self.assertRaisesRegex(ValueError, "license"):
            verify(self.root)

    def test_node_gate_binds_every_executed_npm_file_and_refuses_extras(self):
        sha = lambda data: hashlib.sha256(data).hexdigest()
        self.put("runtime/node.exe", b"node fixture")
        self.put("runtime/LICENSE", b"node notice")
        name = "node_modules/npm/bin/npm-cli.js"
        self.put("runtime/" + name, b"reviewed tool")
        runtime = {"signature_verified": True, "node_sha256": sha(b"node fixture"),
                   "license_sha256": sha(b"node notice"),
                   "npm_inventory": {name: {"bytes": len(b"reviewed tool"), "sha256": sha(b"reviewed tool")}}}
        self.put("tools/licenses/node-runtime.json", json.dumps(runtime).encode())
        self.assertTrue(verify(self.root, node=self.root / "runtime")["node_verified"])
        self.put("runtime/" + name, b"modified tool")
        with self.assertRaisesRegex(ValueError, "npm_tool_bytes"):
            verify(self.root, node=self.root / "runtime")
        self.put("runtime/" + name, b"reviewed tool")
        self.put("runtime/node_modules/npm/extra.js", b"unreviewed")
        with self.assertRaisesRegex(ValueError, "npm_tool_extra"):
            verify(self.root, node=self.root / "runtime")


if __name__ == "__main__":
    unittest.main()
