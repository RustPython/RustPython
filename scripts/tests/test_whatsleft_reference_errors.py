"""Tests for reference-signature error accounting in whats_left."""

import ast
import io
import json
import os
import pathlib
import tempfile
import types
import unittest
from contextlib import redirect_stdout

SOURCE = pathlib.Path(__file__).resolve().parents[1] / "whats_left.py"
tree = ast.parse(SOURCE.read_text(encoding="utf-8"))
names = {"attr_is_not_inherited", "compare"}
nodes = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in names]
namespace = {}
exec(compile(ast.Module(nodes, type_ignores=[]), str(SOURCE), "exec"), namespace)


class ReferenceErrorTests(unittest.TestCase):
    def run_comparison(self, *, module_error):
        builtin = {"sig": "ValueError('builtin')", "sig_error": True, "doc": None}
        reference = {
            "sig": "ValueError('module')" if module_error else "(x, /)",
            "sig_error": module_error,
            "doc": "same",
        }
        actual = {"sig": "(x, /)", "sig_error": False, "doc": "same"}
        with tempfile.TemporaryDirectory() as tmp:
            pathlib.Path(tmp, "demo.py").touch()
            namespace.update(
                expected_methods={"demo": (type, builtin, [])},
                cpymods={"demo": {"f": reference}},
                libdir=os.fsencode(tmp),
                dir_of_mod_or_error=lambda name: {"f": actual},
                import_module=lambda name: types.ModuleType(name),
            )
            output = io.StringIO()
            with redirect_stdout(output):
                namespace["compare"]()
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
