"""Preserve two explicit interpreter identity additions during canonical imports."""

import ast
from pathlib import Path

SYSCONFIG_UPSTREAM = "def _get_implementation():\n    return 'Python'\n"
SYSCONFIG_LOCAL = (
    "def _get_implementation():\n"
    "    return 'RustPython'  # XXX: RUSTPYTHON; For site-packages\n"
)
JIT_ERROR = """# NOTE: RUSTPYTHON exceptions
try:
    JitError
except NameError:
    pass
else:
    PYTHON2_EXCEPTIONS += ("JitError",)

"""
EXCEPTION_LOOP = """for excname in PYTHON2_EXCEPTIONS:
    NAME_MAPPING[("exceptions", excname)] = ("builtins", excname)
"""


def _unique_function(contents: str, expected: str) -> bool:
    tree = ast.parse(contents)
    matches = [
        node
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == "_get_implementation"
    ]
    return (
        len(matches) == 1
        and matches[0] in tree.body
        and ast.dump(matches[0]) == ast.dump(ast.parse(expected).body[0])
        and not any(
            (
                isinstance(node, ast.Name)
                and node.id == "_get_implementation"
                and isinstance(node.ctx, (ast.Store, ast.Del))
            )
            or (isinstance(node, ast.ClassDef) and node.name == "_get_implementation")
            or (
                isinstance(node, ast.alias)
                and (node.asname or node.name.split(".")[0]) == "_get_implementation"
            )
            for node in ast.walk(tree)
        )
    )


def _exception_loop(contents: str) -> bool:
    tree = ast.parse(contents)
    matches = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.For)
        and isinstance(node.iter, ast.Name)
        and node.iter.id == "PYTHON2_EXCEPTIONS"
    ]
    return (
        len(matches) == 1
        and matches[0] in tree.body
        and ast.get_source_segment(contents, matches[0]) == EXCEPTION_LOOP.rstrip()
    )


def _jit_block(contents: str) -> bool:
    tree = ast.parse(contents)
    expected = ast.parse(JIT_ERROR).body[0]
    matches = [node for node in ast.walk(tree) if ast.dump(node) == ast.dump(expected)]
    references = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Name) and node.id == "JitError"
    ]
    return len(matches) == 1 and matches[0] in tree.body and len(references) == 1


def preserve_runtime_patch(path: Path, old: str, new: str) -> str:
    """Fail closed if either known local patch or its canonical anchor changes."""
    parts = path.parts
    if path.is_absolute():
        raise ValueError("Runtime patch target must be repository-relative")
    if parts == ("Lib", "sysconfig", "__init__.py"):
        if old.count(SYSCONFIG_LOCAL) != 1 or not _unique_function(
            old, SYSCONFIG_LOCAL
        ):
            raise ValueError("Unrecognized sysconfig implementation identity")
        if (
            new.count(SYSCONFIG_UPSTREAM) != 1
            or not _unique_function(new, SYSCONFIG_UPSTREAM)
            or "RustPython" in new
        ):
            raise ValueError("Unrecognized canonical sysconfig implementation anchor")
        return new.replace(SYSCONFIG_UPSTREAM, SYSCONFIG_LOCAL, 1)
    if parts == ("Lib", "_compat_pickle.py"):
        if (
            old.count(JIT_ERROR + EXCEPTION_LOOP) != 1
            or not _jit_block(old)
            or not _exception_loop(old)
        ):
            raise ValueError("Unrecognized local JitError compatibility mapping")
        if (
            new.count(EXCEPTION_LOOP) != 1
            or not _exception_loop(new)
            or "JitError" in new
        ):
            raise ValueError("Unrecognized canonical exception mapping anchor")
        return new.replace(EXCEPTION_LOOP, JIT_ERROR + EXCEPTION_LOOP, 1)
    return new
