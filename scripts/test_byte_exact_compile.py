"""Controller integrity checks; these do not test interpreter behavior."""

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

import byte_exact_capture
from byte_exact_compile import (
    capture_environment,
    compare_captures,
    differences,
    requests,
    validate_cache_prefix,
)


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.fixture = {
            "name": "ordinary",
            "source": "x = 1\n",
            "mode": "exec",
            "flags": 0,
        }

    def test_empty_corpus_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "empty"):
            requests([], [])

    def test_unknown_selection_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "unknown"):
            requests([self.fixture], ["missing"])

    def test_duplicate_names_are_an_error(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            requests([self.fixture, self.fixture], [])

    def test_explicit_selection_retains_all_optimizations(self):
        corpus = requests([self.fixture], ["ordinary"])
        self.assertEqual([r["optimize"] for r in corpus], [0, 1, 2])
        self.assertTrue(all(r["filename"] == "byte_exact/ordinary.py" for r in corpus))

    def test_ascii_boundary(self):
        with self.assertRaisesRegex(ValueError, "ASCII"):
            requests([{**self.fixture, "source": "x = 'é'"}], [])

    def test_equal_fields(self):
        self.assertEqual(differences({"a": [1, 2]}, {"a": [1, 2]}), [])

    def test_byte_difference_is_retained(self):
        self.assertEqual(differences([1, 128], [1, 0], "marshal"), ["marshal/1"])

    def test_nested_difference_has_single_leaf_path(self):
        self.assertEqual(
            differences({"code": {"flags": 1}}, {"code": {"flags": 2}}), ["/code/flags"]
        )

    def test_length_difference_is_not_normalized(self):
        self.assertEqual(differences([1, 0], [1], "code"), ["code"])

    def test_types_are_not_coerced(self):
        self.assertEqual(differences(True, 1, "constant"), ["constant"])

    def test_cache_prefix_is_explicit_in_sanitized_environment(self):
        with TemporaryDirectory() as directory:
            prefix = validate_cache_prefix(directory)
            env = capture_environment(None, prefix)
            self.assertEqual(env["PYTHONPYCACHEPREFIX"], str(prefix))
            self.assertEqual(env["PYTHONDONTWRITEBYTECODE"], "1")
            self.assertNotIn("RUSTPYTHONPATH", env)

    def test_populated_cache_prefix_is_rejected(self):
        with TemporaryDirectory() as directory:
            (Path(directory) / "existing.pyc").write_bytes(b"cache")
            with self.assertRaisesRegex(ValueError, "empty"):
                validate_cache_prefix(directory)

    def test_runtime_provenance_is_not_a_comparison_field(self):
        reference = {"marshal": [1], "magic": [2], "code": {}, "runtime": "reference"}
        candidate = {**reference, "runtime": "candidate"}
        self.assertEqual(
            compare_captures(reference, candidate),
            {"marshal": [], "magic": [], "code": []},
        )

    def test_runtime_provenance_follows_serialization_and_fields(self):
        events = []
        request = {**self.fixture, "filename": "ordinary.py", "optimize": 0}
        with (
            patch.object(
                byte_exact_capture.marshal,
                "dumps",
                side_effect=lambda *a: events.append("marshal") or b"raw",
            ),
            patch.object(
                byte_exact_capture,
                "describe",
                side_effect=lambda value: events.append("fields") or {},
            ),
            patch.object(
                byte_exact_capture,
                "runtime_provenance",
                side_effect=lambda: events.append("runtime") or {},
            ),
        ):
            byte_exact_capture.capture(request)
        self.assertEqual(events, ["marshal", "fields", "runtime"])


if __name__ == "__main__":
    unittest.main()
