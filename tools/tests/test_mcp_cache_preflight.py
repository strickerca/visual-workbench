"""B8 packaged MCP cache preflight: path identity, scoped consumer guard and dry-run plan."""
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import mcp_cache_preflight as pre
from mcp_cache_preflight import PreflightRefused, ProcessRow, classify_final_path, classify_processes

LAD = r"C:\Users\owner\AppData\Local"
FAMILY = "Vendor.AgentApp_2p2nqsd0c76g0"
REDIRECT = LAD + "\\Packages\\" + FAMILY + "\\LocalCache\\Local"
KEY_A = "a" * 64
KEY_B = "b" * 64
KEY_C = "c" * 64


def form(requested, observed, *, same=True, reparse=False, child=pre.ANCHOR_NAME):
    return classify_final_path(requested, observed, local_app_data=LAD, same_object=same,
                               reparse_point=reparse, allow_redirect_child=child)


class PathIdentityTests(unittest.TestCase):
    def refused(self, code, *args, **kwargs):
        with self.assertRaises(PreflightRefused) as caught:
            form(*args, **kwargs)
        self.assertEqual(caught.exception.code, code)

    def test_exact_and_syntax_only(self):
        self.assertEqual(form(LAD, LAD).form, "exact")
        self.assertEqual(form(LAD, "\\\\?\\" + LAD).form, "syntax_only")
        self.assertEqual(form(LAD + "\\", "\\\\?\\" + LAD).form, "syntax_only")
        self.assertEqual(form("C:\\", "\\\\?\\C:\\").form, "syntax_only")
        self.assertEqual(form(LAD.replace("\\", "/"), LAD).form, "syntax_only")

    def test_case_only(self):
        self.assertEqual(form(LAD, "\\\\?\\c:\\users\\OWNER\\appdata\\local").form, "case_only")

    def test_recorded_msix_redirect_of_anchor_is_same_directory(self):
        # The 2026-10-20 diagnosis: requested 6 components, observed 10, same file id.
        requested = LAD + "\\Visual Workbench"
        observed = "\\\\?\\" + REDIRECT + "\\Visual Workbench"
        self.assertEqual(len(pre.split_dos_path(requested)[1]) + 1, 6)
        self.assertEqual(len(pre.split_dos_path(observed)[1]) + 1, 10)
        result = form(requested, observed)
        self.assertEqual((result.form, result.package_family), ("msix_app_data_redirect", FAMILY))
        deeper = form(requested + "\\mcp-server-bundles-v1\\" + KEY_A,
                      "\\\\?\\" + REDIRECT.lower() + "\\visual workbench\\mcp-server-bundles-v1\\" + KEY_A)
        self.assertEqual(deeper.package_family.upper(), FAMILY.upper())

    def test_genuine_changes_still_refuse(self):
        requested = LAD + "\\Visual Workbench"
        self.refused("object_identity_changed", requested, "\\\\?\\" + REDIRECT + "\\Visual Workbench", same=False)
        self.refused("object_identity_changed", LAD, LAD, same=False)
        self.refused("reparse_point", requested, requested, reparse=True)
        self.refused("canonical_path_changed", requested, REDIRECT + "\\Other")
        self.refused("canonical_path_changed", requested, LAD + "\\Elsewhere")
        self.refused("canonical_path_changed", requested, "D:" + requested[2:])
        self.refused("canonical_path_changed", requested, LAD + "\\Packages\\not a family\\LocalCache\\Local\\Visual Workbench")
        self.refused("canonical_path_changed", requested, LAD + "\\Packages\\" + FAMILY + "\\LocalCache\\Roaming\\Visual Workbench")
        self.refused("canonical_path_changed", requested, LAD + "\\Packages\\" + FAMILY + "\\Extra\\LocalCache\\Local\\Visual Workbench")
        # Redirection is admitted only for the named anchor child.
        self.refused("canonical_path_changed", LAD + "\\Other", REDIRECT + "\\Other")
        self.refused("canonical_path_changed", requested, "\\\\?\\" + REDIRECT + "\\Visual Workbench", child=None)
        # Above local app data nothing may move.
        self.refused("canonical_path_changed", r"C:\Users\owner", r"C:\Users\other")

    def test_namespaces_and_short_names(self):
        for bad in (r"\\server\share\x", r"\\?\UNC\server\share", r"\\.\C:\x", r"C:\a\..\b", "C:relative",
                    r"C:\a\b:stream", "C:\\a\\\\b", ""):
            with self.assertRaises(PreflightRefused) as caught:
                pre.split_dos_path(bad)
            self.assertEqual(caught.exception.code, "unsupported_namespace")
        self.refused("short_name_unexpanded", r"C:\Users\OWNER~1\AppData\Local", LAD)


class FakeProbe(pre.Probe):
    def __init__(self, objects, long_paths=None):
        # requested path -> (final path, identity, attributes, tag)
        self.objects = {k.upper(): v for k, v in objects.items()}
        self.finals = {pre.split_dos_path(v[0])[1].__repr__().upper(): v[1] for v in objects.values()}
        self.long_paths = long_paths or {}
        self.opened, self.closed = [], False

    def long_path(self, path):
        return self.long_paths.get(path, path)

    def open(self, path):
        key = path.rstrip("\\").upper() if len(path) > 3 else path.upper()
        if key not in self.objects:
            raise FileNotFoundError(path)
        final, identity, attributes, tag = self.objects[key]
        self.opened.append(path)
        return pre.Opened(final, identity[0], identity[1], attributes, tag)

    def reopen_identity(self, path):
        return self.finals[pre.split_dos_path(path)[1].__repr__().upper()]

    def close(self):
        self.closed = True


def namespace(redirect=True, swap_base=False, base_reparse=False, extra=()):
    objects = {}
    parts = pre.split_dos_path(LAD)[1]
    for i in range(len(parts) + 1):
        path = "C:\\" + "\\".join(parts[:i])
        objects[path.rstrip("\\") if i else "C:\\"] = ("\\\\?\\" + path, (7, bytes([i]) * 16), 0x10, 0)
    anchor_real = (REDIRECT if redirect else LAD) + "\\Visual Workbench"
    objects[LAD + "\\Visual Workbench"] = ("\\\\?\\" + anchor_real, (7, b"\x40" * 16), 0x10, 0)
    base_real = anchor_real + "\\mcp-server-bundles-v1"
    if swap_base:
        base_real = LAD + "\\Visual Workbench\\mcp-server-bundles-v1"
    objects[LAD + "\\Visual Workbench\\mcp-server-bundles-v1"] = (
        "\\\\?\\" + base_real, (7, b"\x41" * 16), 0x10 | (0x400 if base_reparse else 0), 0xA0000003 if base_reparse else 0)
    for path, value in extra:
        objects[path] = value
    return objects


class PinTests(unittest.TestCase):
    def test_redirected_chain_pins(self):
        probe = FakeProbe(namespace())
        chain = pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], probe, optional_tail=2)
        self.assertEqual(chain.package_family, FAMILY)
        self.assertEqual([c.form for c in chain.components[-2:]], ["msix_app_data_redirect"] * 2)
        self.assertTrue(all(c.form in ("exact", "syntax_only") for c in chain.components[:-2]))
        self.assertTrue(chain.canonical.endswith("LocalCache\\Local\\Visual Workbench\\mcp-server-bundles-v1"))

    def test_unredirected_chain_pins(self):
        chain = pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], FakeProbe(namespace(False)),
                                        optional_tail=2)
        self.assertIsNone(chain.package_family)

    def test_descendant_in_other_package_refuses(self):
        objects = namespace()
        other = LAD + r"\Packages\Other.App_0123456789abc\LocalCache\Local\Visual Workbench\mcp-server-bundles-v1"
        objects[LAD + r"\Visual Workbench\mcp-server-bundles-v1"] = ("\\\\?\\" + other, (7, b"\x41" * 16), 0x10, 0)
        with self.assertRaises(PreflightRefused) as caught:
            pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], FakeProbe(objects))
        self.assertEqual(caught.exception.code, "canonical_path_changed")

    def test_descendant_leaving_redirect_refuses(self):
        with self.assertRaises(PreflightRefused) as caught:
            pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], FakeProbe(namespace(swap_base=True)))
        self.assertEqual((caught.exception.code, caught.exception.phase), ("canonical_path_changed", "pin_cache_ancestors"))

    def test_reparse_and_identity_refuse(self):
        with self.assertRaises(PreflightRefused) as caught:
            pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], FakeProbe(namespace(base_reparse=True)))
        self.assertEqual(caught.exception.code, "reparse_point")
        objects = namespace()
        final, _, attributes, tag = objects[LAD + "\\Visual Workbench"]
        objects[LAD + "\\Visual Workbench"] = (final, (7, b"\x99" * 16), attributes, tag)
        probe = FakeProbe(objects)
        probe.finals[repr(pre.split_dos_path(final)[1]).upper()] = (7, b"\x40" * 16)
        with self.assertRaises(PreflightRefused) as caught:
            pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], probe)
        self.assertEqual(caught.exception.code, "object_identity_changed")

    def test_absent_optional_tail_and_required_component(self):
        objects = namespace()
        del objects[LAD + "\\Visual Workbench\\mcp-server-bundles-v1"]
        self.assertIsNone(pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], FakeProbe(objects),
                                                  optional_tail=2))
        with self.assertRaises(PreflightRefused) as caught:
            pre.pin_cache_ancestors(LAD, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME], FakeProbe(objects))
        self.assertEqual(caught.exception.code, "component_absent")

    def test_short_local_app_data_is_expanded_first(self):
        short = r"C:\Users\OWNER~1\AppData\Local"
        chain = pre.pin_cache_ancestors(short, [pre.ANCHOR_NAME, pre.CACHE_BASE_NAME],
                                        FakeProbe(namespace(), {short: LAD}), optional_tail=2)
        self.assertEqual(chain.package_family, FAMILY)


CACHE = REDIRECT + "\\Visual Workbench\\mcp-server-bundles-v1"


def row(pid, ppid, name, exe, created=100):
    return ProcessRow(pid, ppid, name, exe, created)


class ConsumerGuardTests(unittest.TestCase):
    def guard(self, rows, roots=(1,), packaged=(CACHE,)):
        return classify_processes(rows, owned_roots=roots, packaged_roots=packaged)

    def test_unrelated_owner_node_tools_do_not_block(self):
        rows = [row(1, 0, "python.exe", r"C:\Python\python.exe")]
        rows += [row(100 + i, 50, "node.exe", r"C:\Program Files\nodejs\node.exe") for i in range(49)]
        result = self.guard(rows)
        self.assertEqual(result["status"], "clear")
        self.assertEqual(result["unrelated_runtime_named_processes_ignored"], 49)

    def test_packaged_runtime_instance_refuses_in_either_path_form(self):
        for base in (CACHE, LAD + "\\Visual Workbench\\mcp-server-bundles-v1"):
            exe = "\\\\?\\" + base.upper() + "\\" + KEY_A + "\\runtime\\node.exe"
            result = self.guard([row(9, 4, "node.exe", exe)],
                                packaged=(CACHE, LAD + "\\Visual Workbench\\mcp-server-bundles-v1"))
            self.assertEqual(result["conflicts"], [{"pid": 9, "name": "node.exe", "reason": "packaged_runtime_in_use"}])
        host = self.guard([row(9, 4, "vw-mcp-host.exe", CACHE + "\\" + KEY_A + "\\runtime\\vw-mcp-host.exe")])
        self.assertEqual(host["status"], "refused")
        image = self.guard([row(9, 4, "node.exe", r"D:\image\node.exe")], packaged=(CACHE, r"D:\image"))
        self.assertEqual(image["status"], "refused")

    def test_task_owned_tree_node_refuses_and_pid_reuse_is_not_owned(self):
        rows = [row(1, 0, "powershell.exe", r"C:\ps.exe", 100), row(2, 1, "cmd.exe", r"C:\cmd.exe", 110),
                row(3, 2, "node.exe", r"C:\nodejs\node.exe", 120)]
        self.assertEqual(self.guard(rows)["conflicts"][0]["reason"], "task_owned_runtime_alive")
        reused = [row(1, 0, "powershell.exe", r"C:\ps.exe", 100), row(3, 1, "node.exe", r"C:\nodejs\node.exe", 50)]
        self.assertEqual(self.guard(reused)["status"], "clear")

    def test_unreadable_runtime_image_is_ambiguous(self):
        result = self.guard([row(5, 4, "node.exe", None), row(6, 4, "svchost.exe", None)])
        self.assertEqual([c["reason"] for c in result["conflicts"]], ["runtime_named_image_ambiguous"])
        unc = self.guard([row(5, 4, "node.exe", r"\\server\share\node.exe")])
        self.assertEqual([c["reason"] for c in unc["conflicts"]], ["runtime_named_image_ambiguous"])

    def test_workbench_app_anywhere_refuses_unless_explicitly_excluded(self):
        rows = [row(1, 0, "python.exe", r"C:\Python\python.exe"),
                row(20, 3, "VisualWorkbench.exe", r"C:\Program Files\Visual Workbench\VisualWorkbench.exe", 100),
                row(21, 20, "node.exe", r"C:\Program Files\nodejs\node.exe", 110),
                row(22, 3, "VisualWorkbenchDev.exe", r"D:\build\VisualWorkbenchDev.exe", 100)]
        result = self.guard(rows)
        self.assertTrue(result["workbench_app_running"])
        self.assertEqual({(c["pid"], c["reason"]) for c in result["conflicts"]},
                         {(20, "workbench_app_running"), (21, "workbench_app_child_runtime"),
                          (22, "workbench_app_running")})
        excluded = classify_processes(rows, owned_roots=(1,), packaged_roots=(CACHE,), excluded_pids=(20, 22))
        self.assertEqual((excluded["status"], excluded["workbench_app_running"]), ("clear", False))

    def test_workbench_runtime_binary_anywhere_refuses(self):
        for name in ("vw-mcp-host.exe", "VW-CODEX-HOST.EXE", "vw-mcp-package.exe", "vw-mcp.exe"):
            result = self.guard([row(9, 4, name, "E:\\elsewhere\\" + name)])
            self.assertEqual(result["conflicts"][0]["reason"], "workbench_runtime_named_process")

    def test_prefix_is_component_wise(self):
        lookalike = CACHE + "-old\\" + KEY_A + "\\runtime\\node.exe"
        self.assertEqual(self.guard([row(9, 4, "node.exe", lookalike)])["status"], "clear")


def entry(name, *, owner=True, ready=True, runtime=True, plain=True, unexpected=False, modified=1_700_000_000):
    return pre.CacheEntry(name, plain, owner, ready, runtime, unexpected, modified)


class PlanTests(unittest.TestCase):
    def test_full_cache_proposes_minimum_task_owned_retirement(self):
        keys = [f"{i:x}" * 64 for i in range(1, 9)]
        entries = [entry(k, modified=1_700_000_000 + i) for i, k in enumerate(keys)]
        entries[5] = entry(keys[5], ready=False, modified=1_700_000_005)
        plan = pre.plan_cache_entries(entries, requested_key=KEY_C, in_use=set())
        self.assertFalse(plan["mutation_performed"])
        self.assertEqual(plan["slots_needed"], 1)
        self.assertEqual(plan["proposed_for_owner_review"], [keys[5]])  # incomplete first
        self.assertTrue(plan["satisfiable_from_task_owned_candidates"])

    def test_protected_entries_are_never_proposed(self):
        entries = [entry(KEY_A, modified=1), entry(KEY_B, owner=False, modified=2), entry("not-a-key", modified=3),
                   entry("d" * 64, plain=False, modified=4), entry("e" * 64, unexpected=True, modified=5),
                   entry("f" * 64, modified=6), entry("1" * 64, modified=7), entry("2" * 64, modified=8)]
        plan = pre.plan_cache_entries(entries, requested_key=KEY_C, in_use={KEY_A})
        states = {r["key"]: r["state"] for r in plan["entries"]}
        self.assertEqual(states[KEY_A], "in_use_keep")
        for key in (KEY_B, "not-a-key", "d" * 64, "e" * 64):
            self.assertEqual(states[key], "not_task_owned_keep")
        self.assertEqual(plan["proposed_for_owner_review"], ["f" * 64])

    def test_requested_key_present_needs_no_slot(self):
        entries = [entry(f"{i:x}" * 64) for i in range(1, 9)]
        plan = pre.plan_cache_entries(entries, requested_key="1" * 64, in_use=set())
        self.assertEqual((plan["slots_needed"], plan["proposed_for_owner_review"], plan["owner_action"]), (0, [], "none"))
        self.assertEqual({r["state"] for r in plan["entries"] if r["key"] == "1" * 64}, {"requested_current_keep"})

    def test_running_workbench_blocks_incomplete_proposal(self):
        keys = [f"{i:x}" * 64 for i in range(1, 9)]
        entries = [entry(k, modified=1_700_000_000 + i) for i, k in enumerate(keys)]
        entries[0] = entry(keys[0], ready=False, modified=1_700_000_000)
        plan = pre.plan_cache_entries(entries, requested_key=KEY_C, in_use=set(), workbench_running=True)
        states = {r["key"]: r["state"] for r in plan["entries"]}
        self.assertEqual(states[keys[0]], "incomplete_possible_live_extraction_keep")
        self.assertNotIn(keys[0], plan["proposed_for_owner_review"])
        self.assertTrue(plan["workbench_app_running"])

    def test_requested_key_present_but_incomplete_needs_owner_action(self):
        for broken in (entry(KEY_C, ready=False), entry(KEY_C, owner=False), entry(KEY_C, unexpected=True),
                       entry(KEY_C, runtime=False)):
            plan = pre.plan_cache_entries([entry(KEY_A), broken], requested_key=KEY_C, in_use=set())
            self.assertEqual(plan["slots_needed"], 0)
            self.assertTrue(plan["owner_action_required"])
            self.assertNotEqual(plan["owner_action"], "none")
            self.assertIn("requested_current_incomplete_owner_action", {r["state"] for r in plan["entries"]})
        healthy = pre.plan_cache_entries([entry(KEY_C)], requested_key=KEY_C, in_use=set())
        self.assertFalse(healthy["owner_action_required"])

    def test_unsatisfiable_and_invalid_key(self):
        entries = [entry(f"{i:x}" * 64, owner=False) for i in range(1, 9)]
        self.assertFalse(pre.plan_cache_entries(entries, requested_key=None, in_use=set())[
            "satisfiable_from_task_owned_candidates"])
        with self.assertRaises(PreflightRefused):
            pre.plan_cache_entries(entries, requested_key="XYZ", in_use=set())


class ScanAndRunTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name) / "mcp-server-bundles-v1"
        self.base.mkdir()

    def make(self, key, owner=True, ready=True, owner_bytes=None):
        folder = self.base / key
        (folder / "runtime").mkdir(parents=True)
        if owner:
            (folder / "owner").write_bytes(owner_bytes or (pre.OWNER_MARKER_PREFIX + key + "\n").encode())
        if ready:
            (folder / "ready").write_bytes(b"manifest copy is never read")
        return folder

    def test_scan_reads_markers_without_mutation(self):
        self.make(KEY_A)
        self.make(KEY_B, ready=False)
        self.make(KEY_C, owner_bytes=b"VisualWorkbench MCP server v1\n" + KEY_A.encode() + b"\n")
        (self.base / "stray.txt").write_bytes(b"x")
        before = sorted(p.relative_to(self.base).as_posix() for p in self.base.rglob("*"))
        found = {e.name: e for e in pre.scan_cache(self.base)}
        self.assertTrue(found[KEY_A].owner_marker_valid and found[KEY_A].ready_present)
        self.assertFalse(found[KEY_B].ready_present)
        self.assertFalse(found[KEY_C].owner_marker_valid)  # marker names another key
        self.assertFalse(found["stray.txt"].plain_directory)
        self.assertEqual(before, sorted(p.relative_to(self.base).as_posix() for p in self.base.rglob("*")))

    def test_run_composes_redirected_pin_guard_and_plan(self):
        for i in range(1, 9):
            self.make(f"{i:x}" * 64)
        probe = FakeProbe(namespace())
        rows = [row(1, 0, "python.exe", r"C:\Python\python.exe")] + [
            row(100 + i, 50, "node.exe", r"C:\Program Files\nodejs\node.exe") for i in range(49)]
        receipt = pre.run(LAD, KEY_C, [1], [], probe, lambda: rows, scan=lambda _: pre.scan_cache(self.base))
        self.assertTrue(probe.closed)
        self.assertEqual(receipt["status"], "ready_for_owner_review")
        self.assertEqual(receipt["path_identity"]["package_family"], FAMILY)
        self.assertTrue(receipt["path_identity"]["cache_base"].startswith("%LOCALAPPDATA%\\Packages\\"))
        self.assertEqual(receipt["cache_plan"]["slots_needed"], 1)
        self.assertEqual(len(receipt["cache_plan"]["proposed_for_owner_review"]), 1)
        self.assertFalse(receipt["mutation_performed"] or receipt["smoke_executed"])

    def test_run_marks_live_runtime_entry_in_use_and_refuses(self):
        for i in range(1, 9):
            self.make(f"{i:x}" * 64)
        live = CACHE + "\\" + "1" * 64 + "\\runtime\\node.exe"
        receipt = pre.run(LAD, KEY_C, [1], [], FakeProbe(namespace()), lambda: [row(9, 4, "node.exe", live)],
                          scan=lambda _: pre.scan_cache(self.base))
        self.assertEqual(receipt["status"], "refused")
        states = {r["key"]: r["state"] for r in receipt["cache_plan"]["entries"]}
        self.assertEqual(states["1" * 64], "in_use_keep")
        self.assertNotIn("1" * 64, receipt["cache_plan"]["proposed_for_owner_review"])

    def test_run_turns_later_failures_into_refusal_receipts(self):
        def snapshot_refused():
            raise PreflightRefused("process_snapshot_incomplete", "consumer_guard")

        def scan_oserror(_):
            raise PermissionError(5, "denied")

        def scan_refused(_):
            raise PreflightRefused("cache_listing_unbounded", "scan_cache")

        cases = [(snapshot_refused, pre.scan_cache, "consumer_guard", "process_snapshot_incomplete"),
                 (lambda: [], scan_oserror, "scan_cache", "os_error"),
                 (lambda: [], scan_refused, "scan_cache", "cache_listing_unbounded")]
        for processes, scan, phase, code in cases:
            probe = FakeProbe(namespace())
            receipt = pre.run(LAD, KEY_C, [1], [], probe, processes, scan=scan)
            self.assertEqual((receipt["status"], receipt["phase"], receipt["refusal_code"]), ("refused", phase, code))
            self.assertTrue(probe.closed)
            self.assertFalse(receipt["mutation_performed"])

    def test_run_tolerates_unc_and_device_images(self):
        self.make("1" * 64)
        rows = [row(9, 4, "node.exe", r"\\server\share\node.exe"), row(10, 4, "vw-mcp-host.exe", r"\\.\device\x.exe"),
                row(11, 4, "other.exe", r"\\?\UNC\server\share\other.exe")]
        receipt = pre.run(LAD, KEY_C, [1], [], FakeProbe(namespace()), lambda: rows,
                          scan=lambda _: pre.scan_cache(self.base))
        self.assertEqual(receipt["status"], "refused")
        self.assertEqual({c["pid"] for c in receipt["consumer_guard"]["conflicts"]}, {9, 10})

    def test_snapshot_walk_must_end_with_no_more_files(self):
        pre._require_complete_walk(18, 5)
        for error, count in ((5, 5), (0, 5), (18, 0)):
            with self.assertRaises(PreflightRefused) as caught:
                pre._require_complete_walk(error, count)
            self.assertEqual(caught.exception.code, "process_snapshot_incomplete")

    def test_receipt_is_written_new_and_parseable(self):
        import json
        out = Path(self.temp.name) / "receipts"
        first = pre.write_receipt({"status": "refused", "package_family": None}, out)
        second = pre.write_receipt({"status": "clear"}, out)
        self.assertNotEqual(first, second)
        self.assertEqual(json.loads(first.read_text(encoding="utf-8"))["status"], "refused")
        self.assertEqual(pre.RECEIPT_DIR.relative_to(pre.RECEIPT_DIR.parents[1]).as_posix(), ".local/mcp-cache-preflight")

    def test_run_reports_pin_refusal_and_absent_namespace(self):
        refused = pre.run(LAD, None, [1], [], FakeProbe(namespace(swap_base=True)), lambda: [])
        self.assertEqual((refused["status"], refused["phase"], refused["refusal_code"]),
                         ("refused", "pin_cache_ancestors", "canonical_path_changed"))
        objects = namespace()
        del objects[LAD + "\\Visual Workbench\\mcp-server-bundles-v1"]
        del objects[LAD + "\\Visual Workbench"]
        self.assertEqual(pre.run(LAD, None, [1], [], FakeProbe(objects), lambda: [])["status"],
                         "cache_absent_in_this_namespace")


@unittest.skipUnless(os.name == "nt", "Windows handle identity")
class WindowsProbeTests(unittest.TestCase):
    """Real handles on a test-created temp tree; nothing outside it is touched."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(os.path.realpath(self.temp.name))

    def test_plain_directory_identity_and_pin(self):
        target = self.root / "Visual Workbench" / "mcp-server-bundles-v1"
        target.mkdir(parents=True)
        probe = pre.WindowsProbe()
        try:
            chain = pre.pin_cache_ancestors(str(self.root), ["Visual Workbench", "mcp-server-bundles-v1"], probe)
        finally:
            probe.close()
        self.assertIsNone(chain.package_family)
        self.assertTrue(all(c.form in ("exact", "syntax_only", "case_only") for c in chain.components))

    def test_case_variant_request_is_same_directory(self):
        (self.root / "Visual Workbench").mkdir()
        probe = pre.WindowsProbe()
        try:
            opened = probe.open(str(self.root / "VISUAL WORKBENCH"))
            same = probe.reopen_identity(opened.final_path) == opened.identity
        finally:
            probe.close()
        result = classify_final_path(str(self.root / "VISUAL WORKBENCH"), opened.final_path,
                                     local_app_data=str(self.root), same_object=same, reparse_point=opened.reparse_point)
        self.assertEqual(result.form, "case_only")

    def test_junction_refuses(self):
        import _winapi
        real = self.root / "real"
        real.mkdir()
        _winapi.CreateJunction(str(real), str(self.root / "Visual Workbench"))
        probe = pre.WindowsProbe()
        try:
            with self.assertRaises(PreflightRefused) as caught:
                pre.pin_cache_ancestors(str(self.root), ["Visual Workbench"], probe)
        finally:
            probe.close()
        self.assertEqual(caught.exception.code, "reparse_point")

    def test_pinned_component_cannot_be_renamed_while_held(self):
        target = self.root / "Visual Workbench"
        target.mkdir()
        probe = pre.WindowsProbe()
        try:
            probe.open(str(target))
            with self.assertRaises(OSError):
                target.rename(self.root / "moved")
        finally:
            probe.close()
        self.assertTrue(target.is_dir())

    def test_process_snapshot_reads_this_process(self):
        rows = pre.windows_processes()
        mine = [r for r in rows if r.pid == os.getpid()]
        self.assertEqual(len(mine), 1)
        self.assertIsNotNone(mine[0].executable)


if __name__ == "__main__":
    unittest.main()
