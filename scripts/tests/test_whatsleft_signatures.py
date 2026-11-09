"""The gate covers comparable signatures, not every scanner diagnostic."""

import copy
import importlib.util
import json
import pathlib
import tempfile
import unittest

PATH = pathlib.Path(__file__).resolve().parents[1] / "check_whatsleft_signatures.py"
spec = importlib.util.spec_from_file_location("signature_gate", PATH)
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def fixture():
    raw = dict(
        status="present",
        kind="method_descriptor",
        binding="unbound",
        value="($self, /)",
        error_type=None,
        representative=True,
    )
    pair = dict(cpython=raw, rustpython=raw.copy(), comparison="exact_raw_match")
    return {
        "raw_text_signatures": {
            "builtins": {"Thing.f": pair},
            "modules": {"demo": {"Thing.f": copy.deepcopy(pair)}},
        },
        "cpython_modules": {
            "demo": {"Thing.f": {"sig": "(self, /)", "sig_error": False}}
        },
        "cpython_builtin_signatures": {
            "Thing": {"type": None, "methods": {"f": "(self, /)"}}
        },
        "mismatched_items": {},
        "missing_items": {},
        "not_implementeds": {},
        "reference_discovery": {"truncated": False},
        "reference_inventory_diagnostics": {"truncated": False, "issues": []},
    }


class GateTests(unittest.TestCase):
    def setUp(self):
        self.result = fixture()
        self.baseline = gate.snapshot(self.result)

    def errors(self):
        return gate.compare(gate.snapshot(self.result), self.baseline)[0]

    def test_roundtrip_and_unchanged(self):
        self.assertEqual(gate.unpack(gate.pack(self.baseline)), self.baseline)
        self.assertEqual(self.errors(), [])
        self.assertEqual(len(self.baseline["coverage"]), 4)

    def test_generated_baseline_roundtrip(self):
        state = {"context": "test", **gate.pack(self.baseline)}
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "baseline.json"
            gate.write_baseline(path, state)
            self.assertEqual(json.loads(path.read_text()), state)

    def test_required_schema(self):
        for field in fixture():
            with self.subTest(field=field):
                result = fixture()
                del result[field]
                with self.assertRaises(KeyError):
                    gate.snapshot(result)

    def test_raw_change(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["rustpython"]["value"] = "($self, value, /)"
        self.assertTrue(self.errors())

    def test_signature_literals_are_not_normalized(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["rustpython"]["value"] = "($self, text=' at 0x123>', /)"
        self.baseline = gate.snapshot(self.result)
        pair["rustpython"]["value"] = "($self, text=' at 0x456>', /)"
        self.assertTrue(self.errors())

    def test_checked_raw_signature_becoming_unobservable_fails(self):
        for side in ("cpython", "rustpython"):
            for status in ("none", "absent", "not_observed", "unsafe_descriptor"):
                with self.subTest(side=side, status=status):
                    result = fixture()
                    result["raw_text_signatures"]["builtins"]["Thing.f"][side][
                        "status"
                    ] = status
                    self.assertTrue(
                        gate.compare(gate.snapshot(result), self.baseline)[0]
                    )

    def test_checked_binding_change_fails(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["rustpython"]["binding"] = "bound_non_none"
        self.assertTrue(self.errors())

    def test_additional_signature_is_unrestricted(self):
        for status in ("none", "absent"):
            with self.subTest(status=status):
                result = fixture()
                pair = result["raw_text_signatures"]["builtins"]["Thing.f"]
                pair["cpython"].update(status=status, value=None)
                result["cpython_builtin_signatures"]["Thing"]["methods"]["f"] = None
                baseline = gate.snapshot(result)
                for value in ("($self, value, /)", "($self, other=None, /)"):
                    pair["rustpython"]["value"] = value
                    self.assertEqual(
                        gate.compare(gate.snapshot(result), baseline)[0], []
                    )

    def test_unrelated_diagnostics_do_not_require_baseline(self):
        self.result.update(
            actual_scan_status={"irrelevant": "scan_returned_exception"},
            uncomparable_signatures={"irrelevant": [["x", "ValueError"]]},
            actual_parent_decisions={"irrelevant": {"x": "isinstance_false"}},
        )
        self.assertEqual(self.errors(), [])

    def test_non_signature_reference_does_not_change_coverage(self):
        self.result["cpython_modules"]["demo"]["noncallable"] = {
            "sig": None,
            "sig_error": False,
        }
        self.assertEqual(self.errors(), [])

    def test_missing_checked_path_fails(self):
        del self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        with self.assertRaises(KeyError):
            self.errors()

    def test_missing_module_observation_fails(self):
        self.result["raw_text_signatures"]["modules"]["demo"]["Thing.f"]["rustpython"][
            "status"
        ] = "not_observed"
        self.assertTrue(self.errors())

    def test_new_comparable_signature_requires_scope_review(self):
        pair = copy.deepcopy(self.result["raw_text_signatures"]["builtins"]["Thing.f"])
        self.result["raw_text_signatures"]["builtins"]["Thing.new"] = pair
        self.assertTrue(self.errors())

    def test_inspect_mismatch(self):
        self.result["mismatched_items"] = {
            "demo": [["demo.Thing.f", "(x)", "(self, /)"]]
        }
        self.assertTrue(self.errors())

    def test_reference_inspection_failure_cannot_look_like_fix(self):
        self.result["cpython_modules"]["demo"]["Thing.f"]["sig_error"] = True
        self.assertTrue(self.errors())

    def test_actual_inspection_failure_cannot_look_like_fix(self):
        self.result["mismatched_items"] = {
            "demo": [["demo.Thing.f", "ValueError('bad')", "(self, /)"]]
        }
        self.assertTrue(self.errors())

    def test_inherited_value_not_invented(self):
        self.result["missing_items"] = {"demo": ["demo.Thing.f (inherited)"]}
        self.assertTrue(self.errors())
        self.result["not_implementeds"] = {"Thing": {"f": "(inherited)"}}
        self.assertTrue(self.errors())

    def test_reference_signature_change_requires_review(self):
        self.result["cpython_modules"]["demo"]["Thing.f"]["sig"] = "(self, value, /)"
        self.assertTrue(self.errors())

    def test_truncation_rejected(self):
        for field in ("reference_discovery", "reference_inventory_diagnostics"):
            with self.subTest(field=field):
                self.result[field]["truncated"] = True
                with self.assertRaises(ValueError):
                    self.errors()
                self.result[field]["truncated"] = False

    def test_reference_ambiguity_rejected(self):
        self.result["reference_inventory_diagnostics"]["issues"] = [
            ["flattened_name_collision", "demo", "Thing.f"]
        ]
        with self.assertRaises(ValueError):
            self.errors()

    def test_existing_missing_signature_remains_a_failure(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["rustpython"].update(status="none", value=None)
        self.baseline = gate.snapshot(self.result)
        self.assertIn("raw:builtin/Thing.f", self.baseline["defects"])
        pair["rustpython"]["status"] = "not_observed"
        self.assertTrue(self.errors())

    def test_existing_inspection_error_remains_a_failure(self):
        self.result["mismatched_items"] = {
            "demo": [["demo.Thing.f", "ValueError('bad')", "(self, /)"]]
        }
        self.baseline = gate.snapshot(self.result)
        self.assertIn("inspect:module/demo/Thing.f", self.baseline["defects"])
        self.result["raw_text_signatures"]["modules"]["demo"]["Thing.f"]["rustpython"][
            "status"
        ] = "not_observed"
        self.result["mismatched_items"] = {}
        self.assertTrue(self.errors())

    def test_empty_inventory_rejected(self):
        self.result["raw_text_signatures"] = {"builtins": {}, "modules": {}}
        self.result["cpython_modules"] = {}
        self.result["cpython_builtin_signatures"] = {}
        with self.assertRaises(ValueError):
            self.errors()

    def test_resolved_failure_requires_baseline_removal(self):
        baseline = copy.deepcopy(self.baseline)
        baseline["defects"]["inspect:module/demo/Thing.f"] = ["(self, /)", "(x)"]
        errors, resolved = gate.compare(self.baseline, baseline)
        self.assertTrue(errors)
        self.assertEqual(resolved, ["inspect:module/demo/Thing.f"])

    def test_changed_known_failure_is_rejected(self):
        self.result["mismatched_items"] = {
            "demo": [["demo.Thing.f", "(x)", "(self, /)"]]
        }
        self.baseline = gate.snapshot(self.result)
        self.result["mismatched_items"]["demo"][0][1] = "(z)"
        self.assertTrue(self.errors())


if __name__ == "__main__":
    unittest.main()
