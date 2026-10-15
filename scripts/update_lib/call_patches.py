"""Preserve the one RustPython-specific environment-reset test argument."""

import ast
from pathlib import Path

CANONICAL_CALL = """assert_python_ok('-c', code, PYTHONPATH="")"""
LOCAL_CALL = """assert_python_ok('-c', code, PYTHONPATH="", RUSTPYTHONPATH="")"""


def _verify_call(contents: str, expected: str) -> None:
    tree = ast.parse(contents)
    classes = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.ClassDef) and node.name == "CmdLineTest"
    ]
    if len(classes) != 1 or classes[0] not in tree.body:
        raise ValueError("Missing or ambiguous CmdLineTest")
    methods = [
        node
        for node in classes[0].body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == "test_empty_PYTHONPATH_issue16309"
    ]
    if len(methods) != 1:
        raise ValueError("Missing or ambiguous environment-reset test")
    target = ast.parse(expected, mode="eval").body
    calls = [
        node
        for node in ast.walk(methods[0])
        if isinstance(node, ast.Call) and ast.dump(node) == ast.dump(target)
    ]
    if (
        len(calls) != 1
        or contents.count(expected) != 1
        or ast.get_source_segment(contents, calls[0]) != expected
    ):
        raise ValueError("Missing or ambiguous environment-reset call")


def preserve_call_patch(relative_path: Path, old: str, canonical: str) -> str:
    if relative_path.parts != ("Lib", "test", "test_cmd_line.py"):
        return canonical
    _verify_call(old, LOCAL_CALL)
    _verify_call(canonical, CANONICAL_CALL)
    return canonical.replace(CANONICAL_CALL, LOCAL_CALL, 1)
