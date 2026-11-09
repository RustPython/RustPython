"""Tests for whats_left's selected top-level reference discovery.

Checks for ambiguity in whats_left's flattened reference inventory.
Tests for the side-effect-conscious raw text-signature observations.
"""

import ast
import builtins
import inspect
import io
import json
import os
import pathlib
import re
import sys
import tempfile
import types
import unittest
import warnings
from contextlib import redirect_stdout

SOURCE = pathlib.Path(__file__).resolve().parents[1] / "whats_left.py"
tree = ast.parse(SOURCE.read_text(encoding="utf-8"))

names = {
    "record_reference_discovery",
    "scan_modules",
    "gen_modules",
    "record_reference_issue",
    "reference_direct_names",
    "ensure_reference_class_scope",
}
nodes = [
    node
    for node in tree.body
    if isinstance(node, ast.FunctionDef) and node.name in names
]
discovery_namespace = {
    "_discovery_type": type,
    "_discovery_str": str,
    "_discovery_len": len,
    "_MAX_DISCOVERY_RECORDS": 16384,
    "_MAX_DISCOVERY_CHARS": 1048576,
    "_reference_type": type,
    "_reference_str": str,
    "_reference_len": len,
    "_reference_set": set,
    "_reference_sum": sum,
    "_reference_any": any,
    "_reference_chars_used": [0],
    "_MAX_REFERENCE_FIELD_CHARS": 4096,
    "_MAX_REFERENCE_TOTAL_CHARS": 32768,
    "_MAX_REFERENCE_ISSUES": 1024,
    "reference_inventory_diagnostics": {"issues": [], "truncated": False},
    "reference_class_selection": {},
    "warnings": warnings,
    "sys": sys,
}
exec(
    compile(ast.Module(nodes, type_ignores=[]), str(SOURCE), "exec"),
    discovery_namespace,
)


class DiscoveryTests(unittest.TestCase):
    def setUp(self):
        self.original = discovery_namespace.copy()
        discovery_namespace.update(
            reference_discovery={
                "names": [],
                "ignored": [],
                "scanner_errors": [],
                "truncated": False,
            },
            _discovery_chars=[0],
        )

    def tearDown(self):
        discovery_namespace.clear()
        discovery_namespace.update(self.original)

    def test_scanner_errors_do_not_change_selection(self):
        class Scanner:
            def run(self, callback, onerror):
                callback(None, "ok", None)
                onerror("broken.child")
                onerror("broken")
                onerror("broken")

        discovery_namespace["ModuleScanner"] = Scanner
        self.assertEqual(discovery_namespace["scan_modules"](), ["ok", "broken"])
        self.assertEqual(
            discovery_namespace["reference_discovery"]["scanner_errors"],
            ["broken.child", "broken", "broken"],
        )

    def test_discovery_records_ignored_and_failed_names(self):
        discovery_namespace.update(
            scan_modules=lambda: ["ok", "bad", "skip"],
            name_sort_key=lambda name: name,
            IGNORED_MODULES={"skip"},
            dir_of_mod_or_error=lambda name, keep_other: (
                ImportError("bad") if name == "bad" else {}
            ),
            import_module=lambda name: types.ModuleType(name),
            BUILTIN_TYPES=[],
        )
        modules, _failed = discovery_namespace["gen_modules"]()
        self.assertEqual(set(modules), {"ok"})
        self.assertEqual(
            discovery_namespace["reference_discovery"]["names"], ["bad", "ok", "skip"]
        )
        self.assertEqual(
            discovery_namespace["reference_discovery"]["ignored"], ["skip"]
        )

    def test_sorted_lookup_precedes_scanner_execution(self):
        events = []

        def original_sorted(items, **kwargs):
            events.append("original")
            return sorted(items, **kwargs)

        def replacement_sorted(items, **kwargs):
            events.append("replacement")
            return sorted(items, **kwargs)

        def scan():
            discovery_namespace["sorted"] = replacement_sorted
            return []

        discovery_namespace.update(
            sorted=original_sorted,
            scan_modules=scan,
            name_sort_key=lambda name: name,
            IGNORED_MODULES=set(),
        )
        discovery_namespace["gen_modules"]()
        self.assertEqual(events, ["original"])

    def test_record_count_and_name_lengths_are_bounded(self):
        discovery_namespace["_MAX_DISCOVERY_RECORDS"] = 1
        record = discovery_namespace["record_reference_discovery"]
        record("scanner_errors", "")
        record("scanner_errors", "")
        self.assertEqual(
            discovery_namespace["reference_discovery"]["scanner_errors"], [""]
        )
        self.assertTrue(discovery_namespace["reference_discovery"]["truncated"])
        discovery_namespace["reference_discovery"] = {
            "names": [],
            "ignored": [],
            "scanner_errors": [],
            "truncated": False,
        }
        discovery_namespace["_discovery_chars"] = [0]
        record("names", "x" * 4097)
        self.assertEqual(discovery_namespace["reference_discovery"]["names"], [])
        self.assertTrue(discovery_namespace["reference_discovery"]["truncated"])

    def test_non_exact_name_is_not_represented_or_reformatted(self):
        class Hostile(str):
            def __repr__(self):
                raise AssertionError("repr called")

            def __str__(self):
                raise AssertionError("str called")

        discovery_namespace["record_reference_discovery"]("names", Hostile("x"))
        self.assertEqual(discovery_namespace["reference_discovery"]["names"], [])
        self.assertTrue(discovery_namespace["reference_discovery"]["truncated"])


names = {
    "record_reference_issue",
    "record_reference_expanded_class",
    "ensure_reference_class_scope",
    "record_reference_class_selection",
    "reference_direct_names",
    "check_reference_class_member",
    "_expand_class_members",
    "gen_methods",
}
nodes = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in names]
inventory_namespace = {
    "_reference_type": type,
    "_reference_str": str,
    "_reference_len": len,
    "_reference_set": set,
    "_reference_sum": sum,
    "_reference_any": any,
    "_reference_chars_used": [0],
    "_MAX_REFERENCE_FIELD_CHARS": 4096,
    "_MAX_REFERENCE_TOTAL_CHARS": 32768,
    "_MAX_REFERENCE_ISSUES": 1024,
    "reference_inventory_diagnostics": {"issues": [], "truncated": False},
    "reference_expanded_classes": {},
    "reference_class_selection": {},
}
exec(
    compile(ast.Module(nodes, type_ignores=[]), str(SOURCE), "exec"),
    inventory_namespace,
)


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.original = inventory_namespace.copy()
        inventory_namespace["reference_inventory_diagnostics"] = {
            "issues": [],
            "truncated": False,
        }
        inventory_namespace["_reference_chars_used"] = [0]
        inventory_namespace["reference_expanded_classes"] = {}
        inventory_namespace["reference_class_selection"] = {}

    def tearDown(self):
        inventory_namespace.clear()
        inventory_namespace.update(self.original)

    def test_direct_dotted_and_generated_collisions(self):
        result = {"A.member": "direct"}
        seen = inventory_namespace["reference_direct_names"](result, "demo")
        check = inventory_namespace["check_reference_class_member"]
        check(seen, "demo", "member", "A.member")
        result["A.member"] = "first"
        check(seen, "demo", "member", "A.member")
        result["A.member"] = "second"
        check(seen, "demo", "nested.member", "A.nested.member")
        result["A.nested.member"] = "third"
        self.assertEqual(result["A.member"], "second")
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [
                ("dotted_module_attribute", "demo", "A.member"),
                ("flattened_name_collision", "demo", "A.member"),
                ("flattened_name_collision", "demo", "A.member"),
                ("dotted_class_attribute", "demo", "A.nested.member"),
            ],
        )

    def test_different_class_paths_collide_after_flattening(self):
        check = inventory_namespace["check_reference_class_member"]
        seen = set()
        check(seen, "demo", "B.C", "A.B.C")
        check(seen, "demo", "C", "A.B.C")
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [
                ("dotted_class_attribute", "demo", "A.B.C"),
                ("flattened_name_collision", "demo", "A.B.C"),
            ],
        )

    def test_non_exact_formatted_key_is_not_hashed(self):
        class Hostile(str):
            def __hash__(self):
                raise AssertionError("hash called")

            def __repr__(self):
                raise AssertionError("repr called")

        inventory_namespace["check_reference_class_member"](
            set(), "demo", "x", Hostile("C.x")
        )
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [("non_exact_flattened_name", "demo", None)],
        )

    def test_non_exact_names_add_no_hash_equality_or_contains_callbacks(self):
        calls = []

        class Hostile(str):
            def __hash__(self):
                calls.append("hash")
                return super().__hash__()

            def __eq__(self, other):
                calls.append("eq")
                return super().__eq__(other)

            def __contains__(self, other):
                calls.append("contains")
                return super().__contains__(other)

            def __repr__(self):
                raise AssertionError("repr called")

        result = {Hostile("C.member"): 1}
        calls.clear()
        inventory_namespace["reference_direct_names"](result, "demo")
        self.assertEqual(calls, [])
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [("non_exact_module_attribute", "demo", None)],
        )

    def test_existing_formatter_caller_and_count_are_preserved(self):
        import sys

        calls = []

        class Hostile(str):
            def __format__(self, spec):
                caller = sys._getframe(1).f_code.co_name
                calls.append(caller)
                return caller

            def __repr__(self):
                raise AssertionError("repr called")

        class DecimalType:
            pass

        module = types.SimpleNamespace(DecimalType=DecimalType)
        result = {"DecimalType": {}}
        inventory_namespace.update(
            BUILTIN_TYPES=[], own_attrs=lambda obj: [(Hostile("x"), 1)]
        )
        inventory_namespace["_expand_class_members"](result, module)
        self.assertEqual(calls, ["_expand_class_members"])
        self.assertEqual(result["DecimalType._expand_class_members"], 1)
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [("non_exact_class_name_or_attribute", "decimal", None)],
        )

    def test_hostile_formatter_cannot_change_original_rhs_value(self):
        import sys

        class Hostile(str):
            def __format__(self, spec):
                sys._getframe(1).f_locals["info"] = "mutated"
                return "x"

        class DecimalType:
            pass

        module = types.SimpleNamespace(DecimalType=DecimalType)
        result = {"DecimalType": {}}
        inventory_namespace.update(
            BUILTIN_TYPES=[], own_attrs=lambda obj: [(Hostile("x"), "original")]
        )
        inventory_namespace["_expand_class_members"](result, module)
        self.assertEqual(result["DecimalType.x"], "original")
        self.assertTrue(
            inventory_namespace["reference_inventory_diagnostics"]["issues"]
        )

    def test_non_exact_scope_is_reported_without_repr(self):
        class Hostile(str):
            def __repr__(self):
                raise AssertionError("repr called")

        inventory_namespace["reference_direct_names"]({}, Hostile("demo"))
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [("non_exact_module_name", None, None)],
        )

    def test_oversized_diagnostics_are_truncated(self):
        inventory_namespace["_MAX_REFERENCE_FIELD_CHARS"] = 2
        inventory_namespace["record_reference_issue"]("large", "demo", "x")
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"],
            {"issues": [], "truncated": True},
        )

    def test_total_text_budget_is_enforced(self):
        inventory_namespace["_MAX_REFERENCE_TOTAL_CHARS"] = 4
        record = inventory_namespace["record_reference_issue"]
        record("one", "d", "xy")
        record("two", "d", "xy")
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"],
            {"issues": [("one", "d", "xy")], "truncated": True},
        )

    def test_decimal_expansion_accounts_for_direct_names(self):
        class DecimalType:
            pass

        DecimalType.__name__ = "DecimalType"
        module = types.SimpleNamespace(DecimalType=DecimalType)
        result = {"DecimalType": {}, "DecimalType.x": "direct"}
        inventory_namespace.update(
            BUILTIN_TYPES=[], own_attrs=lambda obj: [("x", "member")]
        )
        inventory_namespace["_expand_class_members"](result, module)
        self.assertEqual(result["DecimalType.x"], "member")
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
            [
                ("dotted_module_attribute", "decimal", "DecimalType.x"),
                ("flattened_name_collision", "decimal", "DecimalType.x"),
            ],
        )

    def test_reference_class_selection_covers_other_and_dotted_names(self):
        class Empty:
            pass

        module = types.SimpleNamespace(Empty=Empty, plain=1)
        result = {"Empty": {}, "plain": {}, "A.b": {}}
        inventory_namespace.update(BUILTIN_TYPES=[], own_attrs=lambda obj: [])
        inventory_namespace["_expand_class_members"](result, module)
        self.assertEqual(
            inventory_namespace["reference_class_selection"],
            {
                "decimal": {
                    "Empty": "selected",
                    "plain": "not_selected",
                    "A.b": "skipped_dotted",
                }
            },
        )

    def test_zero_member_expanded_class_is_recorded(self):
        class Empty:
            pass

        module = types.SimpleNamespace(Empty=Empty)
        result = {"Empty": {}}
        inventory_namespace.update(BUILTIN_TYPES=[], own_attrs=lambda obj: [])
        inventory_namespace["_expand_class_members"](result, module)
        self.assertEqual(
            inventory_namespace["reference_expanded_classes"], {"decimal": ["Empty"]}
        )
        self.assertEqual(
            inventory_namespace["reference_class_selection"],
            {"decimal": {"Empty": "selected"}},
        )

    def test_duplicate_builtin_names(self):
        inventory_namespace.update(
            BUILTIN_TYPES=[],
            eval=lambda code: list,
            extra_info=lambda obj: {},
            own_attrs=lambda obj: [],
            name_sort_key=lambda name: name,
        )
        inventory_namespace["gen_methods"]()
        self.assertTrue(
            any(
                item[0] == "duplicate_builtin_type"
                for item in inventory_namespace["reference_inventory_diagnostics"][
                    "issues"
                ]
            )
        )

    def test_dotted_builtin_type_is_ambiguous(self):
        unusual = type("A.B", (), {})
        inventory_namespace.update(
            BUILTIN_TYPES=[],
            eval=lambda code: unusual,
            extra_info=lambda obj: {},
            own_attrs=lambda obj: [],
            name_sort_key=lambda name: name,
        )
        inventory_namespace["gen_methods"]()
        self.assertIn(
            ("dotted_builtin_type", "builtins", "A.B"),
            inventory_namespace["reference_inventory_diagnostics"]["issues"],
        )

    def test_non_native_builtin_type_keeps_original_rhs_order(self):
        import sys

        events = []

        class FakeType:
            @property
            def __name__(self):
                events.append("name")
                sys._getframe(1).f_locals["info"] = "mutated"
                return "Repeated"

        typ = FakeType()
        inventory_namespace.update(
            BUILTIN_TYPES=[],
            eval=lambda code: typ,
            extra_info=lambda obj: events.append("info") or {"original": True},
            own_attrs=lambda obj: events.append("attrs") or [],
            name_sort_key=lambda name: name,
        )
        output = inventory_namespace["gen_methods"]()
        self.assertEqual(events, ["info", "attrs", "name"] * (len(events) // 3))
        self.assertIn("'original': True", output)
        self.assertNotIn("mutated", output)
        self.assertTrue(
            any(
                item[0] == "non_native_builtin_type"
                for item in inventory_namespace["reference_inventory_diagnostics"][
                    "issues"
                ]
            )
        )

    def test_rhs_callback_can_replace_type_without_changing_assignment_order(self):
        import sys

        class Hostile:
            @property
            def __name__(self):
                sys._getframe(1).f_locals["info"] = "mutated"
                return "Replacement"

        replacement = Hostile()

        def inspect_type(obj):
            sys._getframe(1).f_locals["typ"] = replacement
            return {"original": True}

        inventory_namespace.update(
            BUILTIN_TYPES=[],
            eval=lambda code: list,
            extra_info=inspect_type,
            own_attrs=lambda obj: [],
            name_sort_key=lambda name: name,
        )
        output = inventory_namespace["gen_methods"]()
        self.assertIn("'original': True", output)
        self.assertNotIn("mutated", output)
        self.assertTrue(
            any(
                item[0] == "non_native_builtin_type"
                for item in inventory_namespace["reference_inventory_diagnostics"][
                    "issues"
                ]
            )
        )

    def test_diagnostics_are_bounded_and_truncation_explicit(self):
        inventory_namespace["_MAX_REFERENCE_ISSUES"] = 1
        record = inventory_namespace["record_reference_issue"]
        record("one", "demo", "x")
        record("two", "demo", "y")
        self.assertEqual(
            inventory_namespace["reference_inventory_diagnostics"],
            {"issues": [("one", "demo", "x")], "truncated": True},
        )


names = {
    "make_native_raw_observer",
    "signature_warnings",
    "attr_is_not_inherited",
    "compare",
    "observe_real_type",
    "observe_real_exception",
    "dir_of_mod_or_error",
    "name_sort_key",
}
nodes = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in names]
raw_namespace = {
    "cpython_failed_imports": {},
    "reference_discovery": {},
    "inspect": inspect,
    "re": re,
    "sys": sys,
    "reference_inventory_diagnostics": {"issues": [], "truncated": False},
    "reference_expanded_classes": {},
    "reference_class_selection": {},
    "actual_parent_scans": {},
    "_safe_type": type,
    "_safe_str": str,
    "_safe_frozenset": frozenset,
}
exec(compile(ast.Module(nodes, type_ignores=[]), str(SOURCE), "exec"), raw_namespace)
raw_namespace["native_raw_text_signature"] = raw_namespace["make_native_raw_observer"]()
for name in ("observe_raw_safely", "extra_info"):
    node = next(
        n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == name
    )
    exec(
        compile(ast.Module([node], type_ignores=[]), str(SOURCE), "exec"), raw_namespace
    )


class RawTextTests(unittest.TestCase):
    def setUp(self):
        original = raw_namespace.copy()
        raw_namespace["actual_parent_scans"] = {}
        raw_namespace["reference_expanded_classes"] = {}

        def restore():
            raw_namespace.clear()
            raw_namespace.update(original)

        self.addCleanup(restore)

    def test_native_text_context_and_noncallables(self):
        observe = raw_namespace["observe_raw_safely"]
        self.assertEqual(observe(len)["binding"], "bound_module")
        self.assertEqual(observe([].append)["binding"], "bound_non_none")
        self.assertEqual(observe(sys.__sizeof__)["binding"], "bound_module")

        class ModuleSubclass(types.ModuleType):
            pass

        self.assertEqual(
            observe(ModuleSubclass("test").__sizeof__)["binding"], "bound_non_none"
        )
        self.assertEqual(observe(list.append)["binding"], "unbound")
        self.assertEqual(observe(list.append)["value"], "($self, object, /)")
        self.assertFalse(observe(list)["representative"])
        self.assertEqual(
            raw_namespace["extra_info"](None)["raw_text_signature"]["status"],
            "not_callable",
        )

    def test_module_binding_compares_only_raw_text(self):
        raw = raw_namespace["observe_raw_safely"](len)
        self.assertEqual(raw["binding"], "bound_module")
        ref = {
            "sig": "(object, /)",
            "sig_error": False,
            "doc": "same",
            "raw_text_signature": raw,
        }
        changed = {**ref, "raw_text_signature": {**raw, "value": "($module, value, /)"}}
        with tempfile.TemporaryDirectory() as tmp:
            pathlib.Path(tmp, "demo.py").touch()
            raw_namespace.update(
                {
                    "expected_methods": {},
                    "cpymods": {"demo": {"func": ref, "same": ref, "instance": ref}},
                    "libdir": os.fsencode(tmp),
                    "dir_of_mod_or_error": lambda name: {
                        "func": changed,
                        "same": ref,
                        "instance": {
                            **ref,
                            "raw_text_signature": raw_namespace["observe_raw_safely"](
                                [].append
                            ),
                        },
                    },
                    "import_module": lambda name: types.ModuleType(name),
                }
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        pair = json.loads(output.getvalue())["raw_text_signatures"]["modules"]["demo"][
            "func"
        ]
        observations = json.loads(output.getvalue())["raw_text_signatures"]["modules"][
            "demo"
        ]
        self.assertEqual(observations["same"]["comparison"], "exact_raw_match")
        self.assertEqual(observations["instance"]["comparison"], "unknown")
        self.assertEqual(pair["comparison"], "raw_difference")
        self.assertEqual(pair["cpython"]["value"], raw["value"])
        self.assertEqual(pair["rustpython"]["value"], "($module, value, /)")

    def test_additional_signature_is_allowed_and_not_reported_as_warning(self):
        raw = raw_namespace["observe_raw_safely"](len)
        ref = {
            "sig": "ValueError('no signature found')",
            "sig_error": True,
            "doc": "same",
            "raw_text_signature": {**raw, "status": "none", "value": None},
        }
        actual = {
            **ref,
            "sig": "(object, /)",
            "sig_error": False,
            "raw_text_signature": raw,
        }
        with tempfile.TemporaryDirectory() as tmp:
            pathlib.Path(tmp, "demo.py").touch()
            raw_namespace.update(
                expected_methods={},
                cpymods={"demo": {"func": ref}},
                libdir=os.fsencode(tmp),
                dir_of_mod_or_error=lambda name: {"func": actual},
                import_module=lambda name: types.ModuleType(name),
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        result = json.loads(output.getvalue())
        pair = result["raw_text_signatures"]["modules"]["demo"]["func"]
        self.assertEqual(pair["comparison"], "additional_signature")
        self.assertFalse(result["mismatched_items"])
        self.assertEqual(raw_namespace["signature_warnings"](result), {})
        self.assertTrue(result["uncomparable_signatures"])
        # A genuine reference observation error must remain visible.
        pair["comparison"] = "unknown"
        self.assertTrue(raw_namespace["signature_warnings"](result))

    def test_real_type_observation_ignores_spoofed_class(self):
        calls = []

        class PretendsToBeType:
            @property
            def __class__(self):
                calls.append("class")
                return type

        obj = PretendsToBeType()
        self.assertTrue(isinstance(obj, type))
        calls.clear()
        self.assertEqual(raw_namespace["observe_real_type"](obj), "not_real_type")
        self.assertEqual(calls, [])
        self.assertEqual(raw_namespace["observe_real_type"](list), "real_type")

    def test_direct_scan_records_selected_parents_without_extra_getters(self):
        calls = []

        class Pretend:
            @property
            def __class__(self):
                calls.append("spoofed_class")
                return type

        class Item:
            pass

        bad = object()
        values = {"Type": Item, "Pretend": Pretend(), "Failure": bad}

        class Module:
            def __dir__(self):
                return list(values)

            def __getattribute__(self, name):
                if name in values:
                    calls.append(name)
                    return values[name]
                return super().__getattribute__(name)

        observe = raw_namespace["observe_real_type"]
        raw_namespace.update(
            import_module=lambda name: Module(),
            extra_info=lambda obj: {"sig": None, "sig_error": False, "doc": None},
            observe_real_type=lambda obj: (
                "classification_failed" if obj is bad else observe(obj)
            ),
            reference_expanded_classes={
                "demo": ["Type", "Pretend", "Failure", "Missing"]
            },
        )
        raw_namespace["dir_of_mod_or_error"]("demo")
        self.assertEqual(calls, ["Failure", "Pretend", "Type"])
        self.assertEqual(
            raw_namespace["actual_parent_scans"],
            {
                "demo": {
                    "Failure": "classification_failed",
                    "Pretend": "not_real_type",
                    "Type": "real_type",
                }
            },
        )

    def test_module_scan_and_second_import_outcomes_are_distinct(self):
        raw = raw_namespace["observe_raw_safely"](list.append)
        ref = {
            "sig": "(self, /)",
            "sig_error": False,
            "doc": "same",
            "raw_text_signature": raw,
        }
        with tempfile.TemporaryDirectory() as tmp:
            pathlib.Path(tmp, "failed.py").touch()
            pathlib.Path(tmp, "demo.py").touch()
            raw_namespace.update(
                expected_methods={},
                cpymods={
                    "absent": {},
                    "failed": {},
                    "demo": {"Thing": ref, "Thing.member": ref},
                },
                libdir=os.fsencode(tmp),
                reference_expanded_classes={
                    "absent": ["Empty"],
                    "failed": ["Empty"],
                    "demo": ["Thing"],
                },
                dir_of_mod_or_error=lambda name: (
                    RuntimeError("failed") if name == "failed" else {"Thing": ref}
                ),
                import_module=lambda name: RuntimeError("second import failed"),
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        result = json.loads(output.getvalue())
        self.assertEqual(
            result["actual_scan_status"],
            {
                "absent": "not_discovered",
                "failed": "scan_returned_exception",
                "demo": "scan_completed",
            },
        )
        self.assertEqual(
            result["actual_second_import_status"],
            {
                "absent": "not_attempted",
                "failed": "not_attempted",
                "demo": "returned_exception",
            },
        )
        self.assertEqual(
            result["actual_parent_decisions"]["demo"],
            {"Thing.member": "isinstance_false"},
        )
        self.assertEqual(
            result["actual_parent_scans"]["absent"], {"Empty": "not_observed"}
        )

    def test_user_getters_and_metaclass_hooks_are_not_called(self):
        class Meta(type):
            def __hash__(self):
                raise AssertionError("hash")

            def __eq__(self, other):
                raise AssertionError("equality")

            @property
            def __mro__(self):
                raise AssertionError("MRO property")

        class Hostile(metaclass=Meta):
            def __getattribute__(self, name):
                raise AssertionError("instance getter")

            def __call__(self):
                pass

        self.assertEqual(
            raw_namespace["observe_raw_safely"](Hostile())["status"],
            "unsafe_or_unrepresentative",
        )

        class HostileList(list, metaclass=Meta):
            pass

        bound = list.append.__get__(HostileList(), HostileList)
        self.assertEqual(
            raw_namespace["observe_raw_safely"](bound)["binding"], "bound_non_none"
        )

    def test_captured_primitives_survive_later_builtin_replacement(self):
        observer = raw_namespace["observe_raw_safely"]
        original = (
            builtins.type,
            builtins.vars,
            builtins.str,
            builtins.tuple,
            builtins.Exception,
        )
        try:

            def fail(*args, **kwargs):
                raise AssertionError("replaced builtin called")

            builtins.type = builtins.vars = builtins.str = builtins.tuple = (
                builtins.Exception
            ) = fail
            result = observer(list.append)
        finally:
            (
                builtins.type,
                builtins.vars,
                builtins.str,
                builtins.tuple,
                builtins.Exception,
            ) = original
        self.assertEqual(result["status"], "present")
        self.assertEqual(result["binding"], "unbound")

    def test_raw_difference_does_not_create_inherited_missing(self):
        class Parent:
            def f(self):
                pass

        class Child(Parent):
            pass

        ref = {
            "status": "present",
            "kind": "method_descriptor",
            "binding": "unbound",
            "value": '($self, text=" at 0x123", /)',
            "error_type": None,
            "representative": None,
        }
        actual = {**ref, "value": '($self, text=" at 0x456", /)'}
        info = {
            "sig": "(self, /)",
            "sig_error": False,
            "doc": "same",
            "raw_text_signature": ref,
        }
        current = {**info, "raw_text_signature": actual}
        with tempfile.TemporaryDirectory() as tmp:
            raw_namespace.update(
                {
                    "expected_methods": {"Child": (Child, info, [("f", info)])},
                    "cpymods": {},
                    "libdir": os.fsencode(tmp),
                    "extra_info": lambda obj: current,
                    "dir_of_mod_or_error": lambda name: {},
                }
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        result = json.loads(output.getvalue())
        self.assertNotIn("Child", result["not_implementeds"])
        self.assertEqual(
            result["raw_text_signatures"]["builtins"]["Child.f"]["comparison"],
            "raw_difference",
        )

    def test_module_inherited_and_missing_observations(self):
        class Parent:
            def same(self):
                pass

            def other(self):
                pass

        class Child(Parent):
            pass

        module = types.ModuleType("demo")
        module.Thing = Child
        raw = {
            "status": "present",
            "kind": "method_descriptor",
            "binding": "unbound",
            "value": '($self, text=" at 0x123", /)',
            "error_type": None,
            "representative": None,
        }
        ref = {
            "sig": "(self, /)",
            "sig_error": False,
            "doc": "same",
            "raw_text_signature": raw,
        }
        actual = {
            **ref,
            "raw_text_signature": {**raw, "value": '($self, text=" at 0x456", /)'},
        }
        incompatible = {**actual, "sig": "(self, required, /)"}

        def inspect_item(obj):
            return incompatible if obj is Parent.other else actual

        with tempfile.TemporaryDirectory() as tmp:
            pathlib.Path(tmp, "demo.py").touch()
            raw_namespace.update(
                {
                    "expected_methods": {},
                    "reference_expanded_classes": {"demo": ["Thing", "Empty"]},
                    "actual_parent_scans": {"demo": {"Thing": "real_type"}},
                    "cpymods": {
                        "demo": {
                            "Thing": ref,
                            "Thing.same": ref,
                            "Thing.other": ref,
                            "Thing.missing": ref,
                        }
                    },
                    "libdir": os.fsencode(tmp),
                    "extra_info": inspect_item,
                    "dir_of_mod_or_error": lambda name: (
                        {"Thing": actual} if name == "demo" else {}
                    ),
                    "import_module": lambda name: module,
                }
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        result = json.loads(output.getvalue())
        raw_result = result["raw_text_signatures"]["modules"]["demo"]
        self.assertEqual(raw_result["Thing.same"]["comparison"], "raw_difference")
        self.assertEqual(
            raw_result["Thing.same"]["cpython"]["value"],
            '($self, text=" at 0x123", /)',
        )
        self.assertEqual(
            raw_result["Thing.other"]["rustpython"]["value"],
            '($self, text=" at 0x456", /)',
        )
        self.assertEqual(
            raw_result["Thing.missing"]["rustpython"]["status"], "not_observed"
        )
        self.assertIn("demo.Thing.other (inherited)", result["missing_items"]["demo"])
        self.assertIn("demo.Thing.missing", result["missing_items"]["demo"])
        self.assertNotIn("demo.Thing.same", result["missing_items"]["demo"])
        self.assertEqual(result["actual_scan_status"]["demo"], "scan_completed")
        self.assertEqual(
            result["actual_second_import_status"]["demo"], "returned_nonexception"
        )
        self.assertEqual(
            result["actual_parent_scans"]["demo"],
            {"Thing": "real_type", "Empty": "not_observed"},
        )
        self.assertEqual(
            result["actual_parent_decisions"]["demo"],
            {
                "Thing.same": "isinstance_true",
                "Thing.other": "isinstance_true",
                "Thing.missing": "isinstance_true",
            },
        )

    def test_constructor_raw_scan_does_not_reinspect_signature(self):
        def forbidden(_obj):
            raise AssertionError("constructor was reinspected")

        raw = raw_namespace["observe_raw_safely"](list)
        with tempfile.TemporaryDirectory() as tmp:
            raw_namespace.update(
                {
                    "expected_methods": {
                        "list": (
                            list,
                            {
                                "sig": None,
                                "sig_error": False,
                                "doc": None,
                                "raw_text_signature": raw,
                            },
                            [],
                        )
                    },
                    "cpymods": {},
                    "libdir": os.fsencode(tmp),
                    "extra_info": forbidden,
                    "dir_of_mod_or_error": lambda name: {},
                }
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        pair = json.loads(output.getvalue())["raw_text_signatures"]["builtins"]["list"]
        self.assertEqual(pair["comparison"], "type_document_exact_raw_match")
        self.assertFalse(pair["cpython"]["representative"])

    def test_reference_empty_modules_have_raw_inventory(self):
        with tempfile.TemporaryDirectory() as tmp:
            raw_namespace.update(
                {
                    "expected_methods": {},
                    "cpymods": {"not_scanned": {}},
                    "libdir": os.fsencode(tmp),
                    "dir_of_mod_or_error": lambda name: {},
                }
            )
            output = io.StringIO()
            with redirect_stdout(output):
                raw_namespace["compare"]()
        result = json.loads(output.getvalue())
        self.assertEqual(result["raw_text_signatures"]["modules"]["not_scanned"], {})


class ReferenceErrorTests(unittest.TestCase):
    def setUp(self):
        global reference_error_namespace
        reference_error_namespace = raw_namespace
        original = raw_namespace.copy()
        reference_error_namespace["actual_parent_scans"] = {}
        reference_error_namespace["reference_expanded_classes"] = {}

        def restore():
            raw_namespace.clear()
            raw_namespace.update(original)

        self.addCleanup(restore)

    def run_comparison(self, *, module_error):
        builtin = {"sig": "ValueError('builtin')", "sig_error": True, "doc": None}
        reference = {
            "sig": "ValueError('module')" if module_error else "(x, /)",
            "sig_error": module_error,
            "doc": "same",
        }
        actual = {"sig": "(x, /)", "sig_error": False, "doc": "same"}
        for info in (builtin, reference, actual):
            info["raw_text_signature"] = reference_error_namespace[
                "observe_raw_safely"
            ](None)
        with tempfile.TemporaryDirectory() as tmp:
            pathlib.Path(tmp, "demo.py").touch()
            reference_error_namespace.update(
                expected_methods={"demo": (type, builtin, [])},
                cpymods={"demo": {"f": reference}},
                libdir=os.fsencode(tmp),
                dir_of_mod_or_error=lambda name: {"f": actual},
                import_module=lambda name: types.ModuleType(name),
            )
            output = io.StringIO()
            with redirect_stdout(output):
                reference_error_namespace["compare"]()
        return json.loads(output.getvalue())

    def test_builtin_name_collision_does_not_hide_clean_module(self):
        result = self.run_comparison(module_error=False)
        self.assertIn("demo", result["implemented"])
        self.assertEqual(
            result["uncomparable_signatures"]["demo"],
            [["demo", "ValueError('builtin')"]],
        )

    def test_module_reference_error_still_prevents_clean_classification(self):
        result = self.run_comparison(module_error=True)
        self.assertNotIn("demo", result["implemented"])
        self.assertIn(
            ["demo.f", "ValueError('module')"],
            result["uncomparable_signatures"]["demo"],
        )


if __name__ == "__main__":
    unittest.main()
