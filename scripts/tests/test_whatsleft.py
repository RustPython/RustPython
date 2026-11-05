"""Tests for whats_left's selected top-level reference discovery."""

import ast
import pathlib
import sys
import types
import unittest
import warnings

SOURCE = pathlib.Path(__file__).resolve().parents[1] / "whats_left.py"
tree = ast.parse(SOURCE.read_text(encoding="utf-8"))
names = {"record_reference_discovery", "scan_modules", "gen_modules"}
nodes = [
    node
    for node in tree.body
    if isinstance(node, ast.FunctionDef) and node.name in names
]
namespace = {
    "_discovery_type": type,
    "_discovery_str": str,
    "_discovery_len": len,
    "_MAX_DISCOVERY_RECORDS": 16384,
    "_MAX_DISCOVERY_CHARS": 1048576,
    "warnings": warnings,
    "sys": sys,
}
exec(compile(ast.Module(nodes, type_ignores=[]), str(SOURCE), "exec"), namespace)


class DiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.original = namespace.copy()
        namespace.update(
            reference_discovery={
                "names": [],
                "ignored": [],
                "scanner_errors": [],
                "truncated": False,
            },
            _discovery_chars=[0],
        )

    def tearDown(self):
        namespace.clear()
        namespace.update(self.original)

    def test_scanner_errors_do_not_change_selection(self):
        class Scanner:
            def run(self, callback, onerror):
                callback(None, "ok", None)
                onerror("broken.child")
                onerror("broken")
                onerror("broken")

        namespace["ModuleScanner"] = Scanner
        self.assertEqual(namespace["scan_modules"](), ["ok", "broken"])
        self.assertEqual(
            namespace["reference_discovery"]["scanner_errors"],
            ["broken.child", "broken", "broken"],
        )

    def test_discovery_records_ignored_and_failed_names(self):
        namespace.update(
            scan_modules=lambda: ["ok", "bad", "skip"],
            name_sort_key=lambda name: name,
            IGNORED_MODULES={"skip"},
            dir_of_mod_or_error=lambda name, keep_other: (
                ImportError("bad") if name == "bad" else {}
            ),
            import_module=lambda name: types.ModuleType(name),
            BUILTIN_TYPES=[],
        )
        modules = namespace["gen_modules"]()
        self.assertEqual(set(modules), {"ok"})
        self.assertEqual(
            namespace["reference_discovery"]["names"], ["bad", "ok", "skip"]
        )
        self.assertEqual(namespace["reference_discovery"]["ignored"], ["skip"])

    def test_sorted_lookup_precedes_scanner_execution(self):
        events = []

        def original_sorted(items, **kwargs):
            events.append("original")
            return sorted(items, **kwargs)

        def replacement_sorted(items, **kwargs):
            events.append("replacement")
            return sorted(items, **kwargs)

        def scan():
            namespace["sorted"] = replacement_sorted
            return []

        namespace.update(
            sorted=original_sorted,
            scan_modules=scan,
            name_sort_key=lambda name: name,
            IGNORED_MODULES=set(),
        )
        namespace["gen_modules"]()
        self.assertEqual(events, ["original"])

    def test_record_count_and_name_lengths_are_bounded(self):
        namespace["_MAX_DISCOVERY_RECORDS"] = 1
        record = namespace["record_reference_discovery"]
        record("scanner_errors", "")
        record("scanner_errors", "")
        self.assertEqual(namespace["reference_discovery"]["scanner_errors"], [""])
        self.assertTrue(namespace["reference_discovery"]["truncated"])
        namespace["reference_discovery"] = {
            "names": [],
            "ignored": [],
            "scanner_errors": [],
            "truncated": False,
        }
        namespace["_discovery_chars"] = [0]
        record("names", "x" * 4097)
        self.assertEqual(namespace["reference_discovery"]["names"], [])
        self.assertTrue(namespace["reference_discovery"]["truncated"])

    def test_non_exact_name_is_not_represented_or_reformatted(self):
        class Hostile(str):
            def __repr__(self):
                raise AssertionError("repr called")

            def __str__(self):
                raise AssertionError("str called")

        namespace["record_reference_discovery"]("names", Hostile("x"))
        self.assertEqual(namespace["reference_discovery"]["names"], [])
        self.assertTrue(namespace["reference_discovery"]["truncated"])


if __name__ == "__main__":
    unittest.main()
