"""Bounded source fixtures, never executing subprocess test bodies."""

import unittest
from pathlib import Path

from update_lib.call_patches import CANONICAL_CALL, LOCAL_CALL, preserve_call_patch

PATH = Path("Lib/test/test_cmd_line.py")
TEMPLATE = """class CmdLineTest:
    def test_empty_PYTHONPATH_issue16309(self):
        result = CALL
        self.assertEqual(result, 'new final assertion')
"""
OLD = TEMPLATE.replace("CALL", LOCAL_CALL).replace(
    "new final assertion", "old assertion"
)
CANONICAL = TEMPLATE.replace("CALL", CANONICAL_CALL)


class CallPatchTests(unittest.TestCase):
    def test_final_body_and_roundtrip(self):
        result = preserve_call_patch(PATH, OLD, CANONICAL)
        self.assertEqual(result, TEMPLATE.replace("CALL", LOCAL_CALL))
        self.assertIn("new final assertion", result)
        self.assertEqual(preserve_call_patch(PATH, result, CANONICAL), result)

    def test_missing_duplicate_owners_and_calls_fail(self):
        for source in (
            CANONICAL.replace("CmdLineTest", "Other"),
            CANONICAL + CANONICAL,
            CANONICAL.replace(
                "        result = ", "        " + CANONICAL_CALL + "\n        result = "
            ),
            CANONICAL.replace("test_empty_PYTHONPATH_issue16309", "other"),
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                preserve_call_patch(PATH, OLD, source)

    def test_changed_keywords_and_decoy_fail(self):
        for old in (
            OLD.replace('RUSTPYTHONPATH=""', 'RUSTPYTHONPATH="changed"'),
            CANONICAL,
            'doc = """' + OLD + '"""\n',
        ):
            with self.subTest(old=old), self.assertRaises(ValueError):
                preserve_call_patch(PATH, old, CANONICAL)
        with self.assertRaises(ValueError):
            preserve_call_patch(
                PATH, OLD, CANONICAL.replace('PYTHONPATH=""', 'PYTHONPATH="changed"')
            )

    def test_unrelated_file_unchanged(self):
        self.assertEqual(
            preserve_call_patch(Path("Lib/test/other.py"), "old", "new"), "new"
        )
