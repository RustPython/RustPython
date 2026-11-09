"""Regression-gate tests use observations, not a second interpreter scan."""

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
        "reference_builtin_inventory": {"Thing": ["f"]},
        "mismatched_items": {},
        "uncomparable_signatures": {},
        "missing_items": {},
        "not_implementeds": {},
        "actual_scan_status": {"demo": "scan_completed"},
        "actual_second_import_status": {"demo": "returned_nonexception"},
        "actual_parent_scans": {"demo": {"Thing": "real_type"}},
        "reference_discovery": {
            "names": ["demo"],
            "truncated": False,
            "scanner_errors": [],
        },
        "actual_parent_decisions": {"demo": {"Thing.f": "isinstance_true"}},
        "reference_class_selection": {"demo": {"Thing": "selected"}},
        "reference_expanded_classes": {"demo": ["Thing"]},
        "cpython_failed_imports": {},
        "reference_inventory_diagnostics": {"issues": [], "truncated": False},
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

    def test_generated_baseline_roundtrip(self):
        state = {"context": "test", **gate.pack(self.baseline)}
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "baseline.json"
            gate.write_baseline(path, state)
            self.assertEqual(json.loads(path.read_text()), state)

    def test_missing_provenance_rejected(self):
        for field in (
            "actual_parent_decisions",
            "reference_class_selection",
            "reference_expanded_classes",
            "cpython_failed_imports",
        ):
            with self.subTest(field=field):
                result = fixture()
                del result[field]
                with self.assertRaises(KeyError):
                    gate.snapshot(result)

    def test_selection_change_not_hidden(self):
        self.result["reference_class_selection"]["demo"]["Thing"] = "not_selected"
        self.assertTrue(self.errors())

    def test_diagnostic_addresses_are_not_regressions(self):
        self.result["uncomparable_signatures"] = {
            "demo": [
                ["demo.f", "ValueError('<type object at 0x123> invalid signature')"]
            ]
        }
        self.baseline = gate.snapshot(self.result)
        self.result["uncomparable_signatures"]["demo"][0][1] = (
            "ValueError('<type object at 0x456> invalid signature')"
        )
        self.assertEqual(self.errors(), [])

    def test_changed_exception_kind_or_message_still_fails(self):
        self.result["uncomparable_signatures"] = {
            "demo": [
                ["demo.f", "ValueError('<type object at 0x123> invalid signature')"]
            ]
        }
        self.baseline = gate.snapshot(self.result)
        for message in (
            "TypeError('<type object at 0x123> invalid signature')",
            "ValueError('<type object at 0x123> failed lookup')",
        ):
            with self.subTest(message=message):
                self.result["uncomparable_signatures"]["demo"][0][1] = message
                self.assertTrue(self.errors())

    def test_inspected_signature_default_addresses_still_fail(self):
        self.result["mismatched_items"] = {
            "demo": [["demo.f", "(x=' at 0x123>')", "(x='expected')"]]
        }
        self.baseline = gate.snapshot(self.result)
        self.result["mismatched_items"]["demo"][0][1] = "(x=' at 0x456>')"
        self.assertTrue(self.errors())

    def test_signature_literals_not_address_normalized(self):
        self.assertEqual(gate.diagnostic("(text=' at 0x123>')"), "(text=' at 0x123>')")
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["comparison"] = "raw_difference"
        pair["rustpython"]["value"] = "($self, text=' at 0x123>', /)"
        self.baseline = gate.snapshot(self.result)
        pair["rustpython"]["value"] = "($self, text=' at 0x456>', /)"
        self.assertTrue(self.errors())

    def test_raw_change(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["comparison"] = "raw_difference"
        pair["rustpython"]["value"] = "($self, value, /)"
        self.assertTrue(self.errors())

    def test_additional_signature_needs_no_baseline_exception(self):
        for reference_status in ("none", "absent"):
            for prefix in ("", "type_document_"):
                with self.subTest(status=reference_status, prefix=prefix):
                    pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
                    pair["cpython"]["status"] = reference_status
                    pair["cpython"]["value"] = None
                    pair["comparison"] = prefix + "different_text_state"
                    self.assertEqual(self.errors(), [])
                    pair["rustpython"]["value"] = "($self, extra=None, /)"
                    self.assertEqual(self.errors(), [])

    def test_missing_reference_signature_is_not_symmetric(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["rustpython"]["status"] = "none"
        pair["rustpython"]["value"] = None
        pair["comparison"] = "different_text_state"
        self.assertTrue(self.errors())

    def test_additional_signature_does_not_hide_unknown_context(self):
        pair = self.result["raw_text_signatures"]["builtins"]["Thing.f"]
        pair["cpython"]["status"] = "none"
        pair["cpython"]["value"] = None
        for comparison in ("unknown", "different_context"):
            with self.subTest(comparison=comparison):
                pair["comparison"] = comparison
                self.assertTrue(self.errors())

    def test_raw_unknown_is_not_a_pass(self):
        self.result["raw_text_signatures"]["builtins"]["Thing.f"]["comparison"] = (
            "unknown"
        )
        self.assertTrue(self.errors())

    def test_type_document_difference_is_not_ignored(self):
        self.result["raw_text_signatures"]["builtins"]["Thing.f"]["comparison"] = (
            "type_document_raw_difference"
        )
        self.assertTrue(self.errors())

    def test_missing_raw_path(self):
        self.result["raw_text_signatures"]["modules"]["demo"].clear()
        self.assertTrue(self.errors())

    def test_new_inventory_requires_review(self):
        self.result["reference_discovery"]["names"].append("new_module")
        self.assertTrue(self.errors())

    def test_reference_coverage_loss(self):
        self.result["cpython_modules"]["demo"].clear()
        self.assertTrue(self.errors())

    def test_inspect_mismatch(self):
        self.result["mismatched_items"] = {"demo": [["demo.f", "(x)", "(y)"]]}
        self.assertTrue(self.errors())

    def test_uncomparable_not_success(self):
        self.result["uncomparable_signatures"] = {"demo": [["demo.f", "ValueError"]]}
        self.assertTrue(self.errors())

    def test_inherited_mismatch_cannot_escape(self):
        self.result["missing_items"] = {"demo": ["demo.Thing.f (inherited)"]}
        self.assertTrue(self.errors())

    def test_import_failure(self):
        self.result["actual_scan_status"]["demo"] = "scan_returned_exception"
        self.assertTrue(self.errors())

    def test_second_import_failure(self):
        self.result["actual_second_import_status"]["demo"] = "returned_exception"
        self.assertTrue(self.errors())

    def test_parent_scan_failure(self):
        self.result["actual_parent_scans"]["demo"]["Thing"] = "not_observed"
        self.assertTrue(self.errors())

    def test_truncation_rejected(self):
        self.result["reference_discovery"]["truncated"] = True
        with self.assertRaises(ValueError):
            self.errors()

    def test_empty_or_missing_schema_rejected(self):
        self.result["raw_text_signatures"]["builtins"].clear()
        with self.assertRaises(ValueError):
            self.errors()
        del self.result["raw_text_signatures"]
        with self.assertRaises(KeyError):
            self.errors()

    def test_resolved_failure_requires_baseline_removal(self):
        baseline = copy.deepcopy(self.baseline)
        baseline["defects"]["inspect:demo/demo.f"] = ["(x)", "(y)"]
        errors, resolved = gate.compare(self.baseline, baseline)
        self.assertTrue(errors)
        self.assertEqual(resolved, ["inspect:demo/demo.f"])

    def test_changed_known_failure_not_suppressed(self):
        self.result["mismatched_items"] = {"demo": [["demo.f", "(x)", "(y)"]]}
        self.baseline = gate.snapshot(self.result)
        self.result["mismatched_items"]["demo"][0][1] = "(z)"
        self.assertTrue(self.errors())


if __name__ == "__main__":
    unittest.main()
