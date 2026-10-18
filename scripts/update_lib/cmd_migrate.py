#!/usr/bin/env python
"""
Migrate test file(s) from CPython, preserving RustPython markers.

Usage:
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py --patches remaining.json
    python scripts/update_lib migrate cpython/Lib/test/test_foo --canonical-removals removals.json --dry-run

Remaining metadata names the exact repository-relative target and its existing
decorator patches. An optional "doctests" object selects existing guarded inputs
by exact source text, mapping each to a single-line reason. Guarded doctests need
this explicit selection; an empty object retires all their guards. Canonical
expected output is never copied from the old file.

Canonical removals are a separate, explicit operation. JSON source metadata
pins path (Lib/test/test_*), commit and tree; removals maps explicit relative
filenames to sha256 and upstream_commit. The caller must first verify and
review historical provenance, including release-branch evidence. This command
checks pinned canonical absence and exact local bytes; it does not authenticate
historical evidence. --dry-run performs all validation without deleting files.

This will:
    1. Extract patches from Lib/test/test_foo.py (if exists)
    2. Apply them to cpython/Lib/test/test_foo.py
    3. Write result to Lib/test/test_foo.py
"""

import argparse
import ast
import builtins
import collections
import copy
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import textwrap

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from update_lib.file_utils import parse_lib_path

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
MULTIPHASE_UPSTREAM = (
    "# gh-116303: Skip test module dependent tests if test modules are unavailable\n"
    'import_helper.import_module("_testmultiphase")\n'
)
MULTIPHASE_LOCAL = """try: # TODO: RUSTPYTHON
    import_helper.import_module("_testmultiphase")
except unittest.SkipTest:
    _testmultiphase = None
"""
LOCK_SPECIALIZATION = """(Frozen_LifetimeTests,
 Source_LifetimeTests
 ) = test_util.test_both(LifetimeTests, init=init)
"""
LOCK_SKIP = """# TODO: RUSTPYTHON; dead weakref module locks not cleaned up in frozen bootstrap
Frozen_LifetimeTests.test_all_locks = unittest.skip("TODO: RUSTPYTHON")(
    Frozen_LifetimeTests.test_all_locks
)
"""
TYPES_DATETIME_GUARD = """# XXX: RUSTPYTHON
try:
    import _datetime
except ImportError:
    _datetime = None
"""
TYPES_UNITTEST_IMPORT = (
    "import unittest  # XXX: RUSTPYTHON; importing to be able to skip tests\n"
)
BYTES_CAPI_UPSTREAM = (
    "    _testlimitedcapi = import_helper.import_module('_testlimitedcapi')\n"
)
BYTES_CAPI_LOCAL = """    # TODO: RUSTPYTHON; import_helper.import_module here cause the entire test stopping
    # _testlimitedcapi = import_helper.import_module('_testlimitedcapi')
    try:
        import _testlimitedcapi
    except ImportError:
        _testlimitedcapi = None
"""
BYTES_ALLOCATION_COMMENTS = {
    "test_alloc": """    # NOTE: RUSTPYTHON:
    #
    # The second instance of self.assertGreater was replaced with
    # self.assertGreaterEqual since, in RustPython, the underlying storage
    # is a Vec which doesn't require trailing null byte.
""",
    "test_init_alloc": """    # NOTE: RUSTPYTHON:
    #
    # The usages of self.assertGreater were replaced with
    # self.assertGreaterEqual since, in RustPython, the underlying storage
    # is a Vec which doesn't require trailing null byte.
""",
}


def _bytes_bindings(root, name):
    """Find bindings conservatively, including nested or indirect shadows."""
    for node in ast.walk(root):
        if isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            bound = node.id
        elif isinstance(node, ast.alias):
            bound = node.asname or node.name.split(".")[0]
        elif isinstance(
            node,
            (
                ast.FunctionDef,
                ast.AsyncFunctionDef,
                ast.ClassDef,
                ast.ExceptHandler,
                ast.MatchAs,
                ast.MatchStar,
            ),
        ):
            bound = node.name
        elif isinstance(node, ast.MatchMapping):
            bound = node.rest
        elif isinstance(node, ast.arg):
            bound = node.arg
        elif isinstance(node, (ast.Global, ast.Nonlocal)):
            bound = name if name in node.names else None
        else:
            continue
        if bound == name:
            yield node


def _bytes_owner(contents):
    tree = ast.parse(contents)
    owner = _unique_anchor(
        contents,
        tree,
        lambda node: isinstance(node, ast.ClassDef) and node.name == "ByteArrayTest",
        direct=True,
    )
    if list(_bytes_bindings(tree, "ByteArrayTest")) != [owner]:
        raise ValueError("Shadowed bytes test class")
    return owner


def _verify_bytes_capi(contents: str, block: str) -> ast.stmt:
    owner = _bytes_owner(contents)
    expected = ast.parse(textwrap.dedent(block)).body[0]
    anchor = _unique_anchor(
        contents, owner, lambda node: ast.dump(node) == ast.dump(expected), direct=True
    )
    allowed = set(ast.walk(anchor))
    if any(node not in allowed for node in _bytes_bindings(owner, "_testlimitedcapi")):
        raise ValueError("Shadowed optional bytes C API import")
    start = anchor.lineno - 1 - (2 if block == BYTES_CAPI_LOCAL else 0)
    anchored_text = "".join(
        contents.splitlines(keepends=True)[start : anchor.end_lineno]
    )
    if start < 0 or anchored_text != block:
        raise ValueError("Missing, changed, or ambiguous optional bytes C API import")
    return anchor


def _preserve_bytes_allocations(old: str, new: str) -> str:
    """Keep only the existing three Vec allocation assertion adaptations."""
    owners = [_bytes_owner(contents) for contents in (old, new)]
    replacements = []
    for name, count in (("test_alloc", 1), ("test_init_alloc", 2)):
        methods = [
            _unique_anchor(
                contents,
                owner,
                lambda node: (
                    isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
                    and node.name == name
                ),
                direct=True,
            )
            for contents, owner in zip((old, new), owners)
        ]
        if any(
            list(_bytes_bindings(owner, name)) != [method]
            for owner, method in zip(owners, methods)
        ):
            raise ValueError("Shadowed bytes allocation test")
        if any(not isinstance(method, ast.FunctionDef) for method in methods):
            raise ValueError("Changed bytes allocation test kind")
        normalized = copy.deepcopy(methods[0])
        expected = ast.parse("self.assertGreaterEqual(alloc, len(b))", mode="eval").body
        matches = [
            node
            for node in ast.walk(normalized)
            if isinstance(node, ast.Call) and ast.dump(node) == ast.dump(expected)
        ]
        if len(matches) != count:
            raise ValueError("Changed local bytes allocation contract")
        for node in matches:
            node.func.attr = "assertGreater"
        if ast.dump(normalized) != ast.dump(methods[1]):
            raise ValueError("Changed canonical bytes allocation contract")
        old_lines = old.splitlines(keepends=True)
        comment = BYTES_ALLOCATION_COMMENTS[name]
        start = methods[0].lineno - 1 - len(comment.splitlines())
        if start < 0 or "".join(old_lines[start : methods[0].lineno - 1]) != comment:
            raise ValueError("Changed local bytes allocation explanation")
        local = "".join(old_lines[start : methods[0].end_lineno])
        replacements.append((methods[1].lineno - 1, methods[1].end_lineno, local))
    lines = new.splitlines(keepends=True)
    for start, end, local in sorted(replacements, reverse=True):
        lines[start:end] = [local]
    result = "".join(lines)
    ast.parse(result)
    return result


def _unique_anchor(contents, root, predicate, *, direct=False, exact=None):
    """Select one real AST anchor, optionally in the immediate owner body."""
    matches = [node for node in ast.walk(root) if predicate(node)]
    if (
        len(matches) != 1
        or (direct and matches[0] not in root.body)
        or (exact is not None and ast.get_source_segment(contents, matches[0]) != exact)
    ):
        raise ValueError("Missing, changed, or ambiguous migration anchor")
    return matches[0]


def _unique_function(contents: str, expected: str) -> bool:
    tree = ast.parse(contents)
    target = ast.parse(expected).body[0]
    node = _unique_anchor(
        contents,
        tree,
        lambda node: (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == target.name
        ),
        direct=True,
    )
    return ast.dump(node) == ast.dump(target) and not any(
        (
            isinstance(node, ast.Name)
            and node.id == target.name
            and isinstance(node.ctx, (ast.Store, ast.Del))
        )
        or (isinstance(node, ast.ClassDef) and node.name == target.name)
        or (
            isinstance(node, ast.alias)
            and (node.asname or node.name.split(".")[0]) == target.name
        )
        for node in ast.walk(tree)
    )


def _exception_loop(contents: str) -> bool:
    _unique_anchor(
        contents,
        ast.parse(contents),
        lambda node: (
            isinstance(node, ast.For)
            and isinstance(node.iter, ast.Name)
            and node.iter.id == "PYTHON2_EXCEPTIONS"
        ),
        direct=True,
        exact=EXCEPTION_LOOP.rstrip(),
    )
    return True


def _jit_block(contents: str) -> bool:
    tree = ast.parse(contents)
    expected = ast.dump(ast.parse(JIT_ERROR).body[0])
    _unique_anchor(contents, tree, lambda node: ast.dump(node) == expected, direct=True)
    return (
        sum(
            isinstance(node, ast.Name) and node.id == "JitError"
            for node in ast.walk(tree)
        )
        == 1
    )


def _preserve_top_level_block(old, new, upstream, local):
    """Require exact, consecutive top-level AST statements and comment bytes."""
    for contents, fragment in ((old, local), (new, upstream)):
        tree, expected = ast.parse(contents), ast.parse(fragment).body
        nodes = [
            _unique_anchor(
                contents,
                tree,
                lambda node: ast.dump(node) == ast.dump(item),
                direct=True,
            )
            for item in expected
        ]
        start = tree.body.index(nodes[0])
        lines = contents.splitlines(keepends=True)
        first = nodes[0].lineno - expected[0].lineno
        if (
            tree.body[start : start + len(nodes)] != nodes
            or contents.count(fragment) != 1
            or "".join(lines[first : nodes[-1].end_lineno]) != fragment
        ):
            raise ValueError("Changed or ambiguous migration customization anchor")
    return new.replace(upstream, local, 1)


def _preserve_multiphase_guard(old, new):
    for contents, local in ((old, True), (new, False)):
        tree = ast.parse(contents)
        if sum(
            isinstance(n, ast.Constant) and n.value == "_testmultiphase"
            for n in ast.walk(tree)
        ) != 1 or sum(
            isinstance(n, ast.Name) and n.id == "_testmultiphase"
            for n in ast.walk(tree)
        ) != int(local):
            raise ValueError("Ambiguous _testmultiphase anchor")
    return _preserve_top_level_block(old, new, MULTIPHASE_UPSTREAM, MULTIPHASE_LOCAL)


def _preserve_frozen_lock_skip(old, new):
    methods = []
    for contents, local in ((old, True), (new, False)):
        tree = ast.parse(contents)
        owner = _unique_anchor(
            contents,
            tree,
            lambda n: isinstance(n, ast.ClassDef) and n.name == "LifetimeTests",
            direct=True,
        )
        method = _unique_anchor(
            contents,
            owner,
            lambda n: isinstance(n, ast.FunctionDef) and n.name == "test_all_locks",
            direct=True,
        )
        methods.append(ast.dump(method))
        if sum(
            isinstance(n, ast.Name) and n.id == "Frozen_LifetimeTests"
            for n in ast.walk(tree)
        ) != (3 if local else 1):
            raise ValueError("Ambiguous frozen lock specialization")
    if methods[0] != methods[1]:
        raise ValueError("Frozen lock test changed; review its customization")
    setup = "def setUpModule():\n    thread_info = threading_helper.threading_setup()\n    unittest.addModuleCleanup(threading_helper.threading_cleanup, *thread_info)\n"
    return _preserve_top_level_block(
        old,
        new,
        LOCK_SPECIALIZATION + "\n\n" + setup,
        LOCK_SPECIALIZATION + "\n" + LOCK_SKIP + "\n" + setup,
    )


def preserve_migration_customizations(path: pathlib.Path, old: str, new: str) -> str:
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
    if parts == ("Lib", "test", "test_cmd_line.py"):
        _verify_call(old, LOCAL_CALL)
        _verify_call(new, CANONICAL_CALL)
        return new.replace(CANONICAL_CALL, LOCAL_CALL, 1)
    if parts == ("Lib", "test", "test_bytes.py"):
        _verify_bytes_capi(old, BYTES_CAPI_LOCAL)
        anchor = _verify_bytes_capi(new, BYTES_CAPI_UPSTREAM)
        lines = new.splitlines(keepends=True)
        lines[anchor.lineno - 1 : anchor.end_lineno] = [BYTES_CAPI_LOCAL]
        return _preserve_bytes_allocations(old, "".join(lines))
    if parts == ("Lib", "test", "test_importlib", "util.py"):
        return _preserve_multiphase_guard(old, new)
    if parts == ("Lib", "test", "test_importlib", "test_locks.py"):
        return _preserve_frozen_lock_skip(old, new)
    if parts == ("Lib", "test", "test_types.py"):
        for contents in (old, new):
            if (
                sum(
                    isinstance(node, ast.alias) and node.name == "_datetime"
                    for node in ast.walk(ast.parse(contents))
                )
                != 1
            ):
                raise ValueError("Ambiguous test_types _datetime import")
        if any(
            isinstance(node, ast.alias) and node.name == "unittest"
            for node in ast.walk(ast.parse(new))
        ):
            raise ValueError("Canonical test_types already imports unittest directly")
        new = _preserve_top_level_block(
            old, new, "import _datetime\n", TYPES_DATETIME_GUARD
        )
        return _preserve_top_level_block(
            old, new, "import typing\n", "import typing\n\n" + TYPES_UNITTEST_IMPORT
        )
    return new


CANONICAL_CALL = """assert_python_ok('-c', code, PYTHONPATH="")"""
LOCAL_CALL = """assert_python_ok('-c', code, PYTHONPATH="", RUSTPYTHONPATH="")"""


def _verify_call(contents: str, expected: str) -> None:
    tree = ast.parse(contents)
    owner = _unique_anchor(
        contents,
        tree,
        lambda node: isinstance(node, ast.ClassDef) and node.name == "CmdLineTest",
        direct=True,
    )
    method = _unique_anchor(
        contents,
        owner,
        lambda node: (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == "test_empty_PYTHONPATH_issue16309"
        ),
        direct=True,
    )
    target = ast.dump(ast.parse(expected, mode="eval").body)
    _unique_anchor(
        contents,
        method,
        lambda node: isinstance(node, ast.Call) and ast.dump(node) == target,
        exact=expected,
    )
    if contents.count(expected) != 1:
        raise ValueError("Ambiguous environment-reset call text")


SYMLINK_UPSTREAM = """            home = os.path.dirname(self.real)
            if sysconfig.is_python_build():
                home = os.path.join(home, sysconfig.get_config_var('VPATH'))
            self._env["PYTHONHOME"] = home
"""
SYMLINK_LEGACY = """            self._env["PYTHONHOME"] = os.path.dirname(self.real)
            if sysconfig.is_python_build():
                self._env["PYTHONPATH"] = STDLIB_DIR
"""
SYMLINK_LOCAL = (
    SYMLINK_UPSTREAM
    + """            # TODO: RUSTPYTHON; Cargo builds do not use CPython's Modules/Setup.
            if sys.implementation.name == 'rustpython':
                self._env["PYTHONPATH"] = STDLIB_DIR
"""
)


def copy_support_python_symlink(source, destination):
    """Adapt only the canonical Windows symlink helper's library environment."""
    owners = []
    for contents in (source, destination):
        tree = ast.parse(contents)
        owner = _unique_anchor(
            contents,
            tree,
            lambda node: (
                isinstance(node, ast.ClassDef) and node.name == "PythonSymlink"
            ),
            direct=True,
        )
        if list(_bytes_bindings(tree, "PythonSymlink")) != [owner]:
            raise ValueError("Shadowed PythonSymlink class")
        owners.append(owner)
    canonical = ast.get_source_segment(source, owners[0]) + "\n"
    current = ast.get_source_segment(destination, owners[1]) + "\n"
    if canonical.count(SYMLINK_UPSTREAM) != 1 or SYMLINK_LOCAL in canonical:
        raise ValueError("Changed canonical PythonSymlink environment anchor")
    platform = _unique_anchor(
        canonical,
        ast.parse(canonical).body[0],
        lambda node: (
            isinstance(node, ast.If)
            and ast.dump(node.test)
            == ast.dump(ast.parse("sys.platform == 'win32'", mode="eval").body)
        ),
        direct=True,
    )
    method = _unique_anchor(
        canonical,
        ast.Module(body=platform.body, type_ignores=[]),
        lambda node: (
            isinstance(node, ast.FunctionDef) and node.name == "_platform_specific"
        ),
        direct=True,
    )
    condition = _unique_anchor(
        canonical,
        method,
        lambda node: (
            isinstance(node, ast.If)
            and ast.dump(node.test)
            == ast.dump(ast.parse("sysconfig.is_python_build()", mode="eval").body)
        ),
        direct=True,
    )
    lines = canonical.splitlines(keepends=True)
    if (
        "".join(lines[condition.lineno - 2 : condition.end_lineno + 1])
        != SYMLINK_UPSTREAM
    ):
        raise ValueError("Changed canonical PythonSymlink environment body")
    adapted = canonical.replace(SYMLINK_UPSTREAM, SYMLINK_LOCAL, 1)
    legacy = canonical.replace(SYMLINK_UPSTREAM, SYMLINK_LEGACY, 1)
    if current not in (canonical, legacy, adapted):
        raise ValueError("Changed local PythonSymlink; review before migration")
    lines = destination.splitlines(keepends=True)
    lines[owners[1].lineno - 1 : owners[1].end_lineno] = [adapted]
    result = "".join(lines)
    ast.parse(result)
    return result


def migrate_support_python_symlink(path, source_hash, destination_hash):
    target = parse_lib_path(path)
    if target.parts != ("Lib", "test", "support", "__init__.py"):
        raise ValueError(
            "PythonSymlink migration requires Lib/test/support/__init__.py"
        )
    source, destination = path.read_bytes(), target.read_bytes()
    for label, contents, expected in (
        ("source", source, source_hash),
        ("destination", destination, destination_hash),
    ):
        if hashlib.sha256(contents).hexdigest() != expected:
            raise ValueError(f"{label} SHA256 does not match the pinned input")
    result = copy_support_python_symlink(
        source.decode("utf-8"), destination.decode("utf-8")
    ).encode("utf-8")
    if target.read_bytes() != destination:
        raise ValueError("Destination changed during PythonSymlink preparation")
    target.write_bytes(result)
    print(
        json.dumps(
            {
                "helper": "PythonSymlink",
                "source_sha256": source_hash,
                "destination_before_sha256": destination_hash,
                "destination_after_sha256": hashlib.sha256(result).hexdigest(),
            },
            sort_keys=True,
        )
    )


def copy_support_assignments(source, destination, names, exports):
    """Copy contiguous canonical assignments and export lines, failing closed.

    Only additions are supported. Dependencies must already be imported with the
    same bindings, or be builtins/earlier selected assignments. The next unchanged
    top-level statement is the insertion anchor; unrelated bytes stay untouched.
    """
    if (
        not names
        or len(set(names)) != len(names)
        or not all(name.isidentifier() for name in names)
    ):
        raise ValueError("Assignment names must be unique identifiers")
    if len(set(exports)) != len(exports) or not set(exports) <= set(names):
        raise ValueError("Exports must be unique selected assignment names")
    src, dst = ast.parse(source), ast.parse(destination)

    def assignment(tree, name):
        matches = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Name)
            and isinstance(node.ctx, ast.Store)
            and node.id == name
        ]
        nodes = [
            node
            for node in tree.body
            if isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == name
        ]
        rebound = any(
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
            and node.name == name
            or isinstance(node, ast.alias)
            and (node.asname or node.name.split(".")[0]) == name
            for node in ast.walk(tree)
        )
        if len(nodes) != 1 or matches != [nodes[0].targets[0]] or rebound:
            raise ValueError(f"Expected one top-level assignment to {name}")
        return nodes[0]

    selected = [assignment(src, name) for name in names]
    start = src.body.index(selected[0])
    if src.body[start : start + len(names)] != selected or start + len(names) == len(
        src.body
    ):
        raise ValueError("Assignments must be contiguous and followed by an anchor")
    if (start and src.body[start - 1].end_lineno >= selected[0].lineno) or (
        selected[-1].end_lineno >= src.body[start + len(names)].lineno
    ):
        raise ValueError("Selected assignments must occupy complete source lines")
    for node in ast.walk(dst):
        if (
            isinstance(node, ast.Name)
            and isinstance(node.ctx, ast.Store)
            and node.id in names
            or isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
            and node.name in names
            or isinstance(node, ast.alias)
            and (node.asname or node.name.split(".")[0]) in names
        ):
            raise ValueError("A selected assignment already exists in the destination")

    anchor = src.body[start + len(names)]
    matches = [node for node in dst.body if ast.dump(node) == ast.dump(anchor)]
    if len(matches) != 1:
        raise ValueError("Expected one unchanged destination anchor")
    for tree, node in ((src, anchor), (dst, matches[0])):
        index = tree.body.index(node)
        if (
            getattr(node, "decorator_list", ())
            or node.col_offset
            or index
            and tree.body[index - 1].end_lineno >= node.lineno
            or index + 1 < len(tree.body)
            and tree.body[index + 1].lineno <= node.end_lineno
        ):
            raise ValueError(
                "Anchor must be undecorated and occupy complete source lines"
            )
    available = set(vars(builtins))
    imports = (set(), set())
    for node in src.body:
        matching = [
            other
            for other in dst.body
            if ast.dump(node) == ast.dump(other) and other.lineno < matches[0].lineno
        ]
        if (
            isinstance(node, (ast.Import, ast.ImportFrom))
            and node.lineno < selected[0].lineno
            and len(matching) == 1
        ):
            imports[0].update(node.names)
            imports[1].update(matching[0].names)
            available.update(
                alias.asname or alias.name.split(".")[0] for alias in node.names
            )
    dependencies = set()
    for name, node in zip(names, selected):
        used = {
            child.id
            for child in ast.walk(node.value)
            if isinstance(child, ast.Name) and isinstance(child.ctx, ast.Load)
        }
        missing = used - available
        if missing:
            raise ValueError(f"Missing dependencies for {name}: {sorted(missing)}")
        dependencies.update(used - set(names))
        available.add(name)
    for tree, boundary, allowed in (
        (src, selected[0].lineno, imports[0]),
        (dst, matches[0].lineno, imports[1]),
    ):
        if dependencies and any(
            isinstance(node, ast.ImportFrom)
            and node.lineno < boundary
            and any(alias.name == "*" for alias in node.names)
            for node in ast.walk(tree)
        ):
            raise ValueError("Wildcard import makes dependency bindings ambiguous")
        for dependency in dependencies:
            bindings = []
            for node in ast.walk(tree):
                if getattr(node, "lineno", boundary) >= boundary:
                    continue
                if isinstance(node, ast.Name) and isinstance(
                    node.ctx, (ast.Store, ast.Del)
                ):
                    name = node.id
                elif isinstance(node, ast.alias):
                    name = node.asname or node.name.split(".")[0]
                elif isinstance(
                    node,
                    (
                        ast.FunctionDef,
                        ast.AsyncFunctionDef,
                        ast.ClassDef,
                        ast.ExceptHandler,
                        ast.MatchAs,
                        ast.MatchStar,
                    ),
                ):
                    name = node.name
                elif isinstance(node, ast.arg):
                    name = node.arg
                elif isinstance(node, ast.MatchMapping):
                    name = node.rest
                else:
                    continue
                if name == dependency:
                    bindings.append(node)
            if len(bindings) > 1 or any(node not in allowed for node in bindings):
                raise ValueError(f"Ambiguous dependency binding: {dependency}")

    source_lines, lines = source.splitlines(True), destination.splitlines(True)
    additions = [
        (
            matches[0].lineno - 1,
            source_lines[selected[0].lineno - 1 : anchor.lineno - 1],
        )
    ]
    if exports:
        src_all, dst_all = assignment(src, "__all__"), assignment(dst, "__all__")
        if not isinstance(src_all.value, ast.List) or not isinstance(
            dst_all.value, ast.List
        ):
            raise ValueError("__all__ must be a literal list")
        if any(
            not isinstance(node, ast.Constant) or not isinstance(node.value, str)
            for node in src_all.value.elts + dst_all.value.elts
        ):
            raise ValueError("__all__ entries must be literal strings")
        src_values, dst_values = (
            ast.literal_eval(src_all.value),
            ast.literal_eval(dst_all.value),
        )
        if any(src_values.count(name) != 1 or name in dst_values for name in exports):
            raise ValueError(
                "Exports must occur once in canonical __all__ and be absent locally"
            )
        export_nodes = [node for node in src_all.value.elts if node.value in exports]
        export_lines = sorted({node.lineno - 1 for node in export_nodes})
        fragment = "".join(source_lines[line] for line in export_lines)
        if ast.literal_eval("[\n" + fragment + "]") != exports:
            raise ValueError(
                "Canonical export lines include unselected entries or differ in order"
            )
        if lines[dst_all.lineno - 1].strip() != "__all__ = [":
            raise ValueError("Expected a multiline destination __all__ list")
        additions.append((dst_all.lineno, [fragment]))
    for offset, fragment in sorted(additions, reverse=True):
        lines[offset:offset] = fragment
    result = "".join(lines)
    ast.parse(result)
    return result


def migrate_support_assignments(path, names, exports, source_hash, destination_hash):
    target = parse_lib_path(path)
    if (
        target.parts[:3] != ("Lib", "test", "support")
        or ".." in target.parts
        or target.suffix != ".py"
    ):
        raise ValueError(
            "Selective assignments are restricted to Lib/test/support Python files"
        )
    source, destination = path.read_bytes(), target.read_bytes()
    for label, contents, expected in (
        ("source", source, source_hash),
        ("destination", destination, destination_hash),
    ):
        if hashlib.sha256(contents).hexdigest() != expected:
            raise ValueError(f"{label} SHA256 does not match the pinned input")
    result = copy_support_assignments(
        source.decode("utf-8"), destination.decode("utf-8"), names, exports
    ).encode("utf-8")
    if target.read_bytes() != destination:
        raise ValueError("Destination changed during assignment preparation")
    target.write_bytes(result)
    print(
        json.dumps(
            {
                "assignments": names,
                "exports": exports,
                "source_sha256": source_hash,
                "destination_before_sha256": destination_hash,
                "destination_after_sha256": hashlib.sha256(result).hexdigest(),
            },
            sort_keys=True,
        )
    )


def patch_single_content(
    src_path: pathlib.Path,
    lib_path: pathlib.Path,
    patches_document: dict | None = None,
) -> str:
    """
    Patch content without writing to disk.

    Args:
        src_path: Source file path (e.g., cpython/Lib/test/foo.py)
        lib_path: Lib path to extract patches from (e.g., Lib/test/foo.py)

    Returns:
        The patched content.
    """
    from update_lib import apply_patches, extract_patches
    from update_lib.patch_spec import apply_doctest_metadata, extract_doctest_guards

    # Extract patches from existing file (if exists)
    if lib_path.exists():
        old_content = lib_path.read_text(encoding="utf-8")
        patches = extract_patches(old_content)
    else:
        patches = {}

    repository = pathlib.Path(__file__).resolve().parents[2]
    try:
        relative_path = lib_path.resolve().relative_to(repository)
    except ValueError:
        # Explicit output paths outside this repository are not its Lib.
        relative_path = pathlib.Path("__external_output__")
    if patches_document is not None:
        from update_lib.patch_spec import select_remaining_patches

        patches = select_remaining_patches(patches, patches_document, relative_path)

    # Apply patches to source content
    src_content = src_path.read_text(encoding="utf-8")
    if lib_path.exists():
        src_content = preserve_migration_customizations(
            relative_path, old_content, src_content
        )
        guards = extract_doctest_guards(old_content)
        selection = patches_document.get("doctests") if patches_document else None
        if guards and selection is None:
            raise ValueError(
                "Existing doctest guards require explicit doctests metadata"
            )
        if selection is not None:
            src_content = apply_doctest_metadata(old_content, src_content, selection)
    if not src_content and not patches:
        return src_content
    return apply_patches(src_content, patches)


def patch_file(
    src_path: pathlib.Path,
    lib_path: pathlib.Path | None = None,
    verbose: bool = True,
    patches_document: dict | None = None,
) -> None:
    """
    Patch a single file from source to lib.

    Args:
        src_path: Source file path (e.g., cpython/Lib/test/foo.py)
        lib_path: Target lib path. If None, derived from src_path.
        verbose: Print progress messages
    """
    if lib_path is None:
        lib_path = parse_lib_path(src_path)

    if lib_path.exists():
        if verbose:
            print(f"Patching: {src_path} -> {lib_path}")
        content = patch_single_content(src_path, lib_path, patches_document)
    else:
        if verbose:
            print(f"Copying: {src_path} -> {lib_path}")
        if patches_document is not None:
            raise ValueError("Marker override requires an existing target")
        content = src_path.read_text(encoding="utf-8")

    lib_path.parent.mkdir(parents=True, exist_ok=True)
    lib_path.write_text(content, encoding="utf-8")


def patch_directory(
    src_dir: pathlib.Path,
    lib_dir: pathlib.Path | None = None,
    verbose: bool = True,
) -> None:
    """
    Patch all files in a directory from source to lib.

    Args:
        src_dir: Source directory path (e.g., cpython/Lib/test/test_foo/)
        lib_dir: Target lib directory. If None, derived from src_dir.
        verbose: Print progress messages
    """
    if lib_dir is None:
        lib_dir = parse_lib_path(src_dir)

    src_files = sorted(
        f
        for f in src_dir.glob("**/*")
        if f.is_file() and "__pycache__" not in f.relative_to(src_dir).parts[:-1]
    )

    for src_file in src_files:
        rel_path = src_file.relative_to(src_dir)
        lib_file = lib_dir / rel_path

        if src_file.suffix == ".py":
            if lib_file.exists():
                if verbose:
                    print(f"Patching: {src_file} -> {lib_file}")
                content = patch_single_content(src_file, lib_file)
            else:
                if verbose:
                    print(f"Copying: {src_file} -> {lib_file}")
                content = src_file.read_text(encoding="utf-8")

            lib_file.parent.mkdir(parents=True, exist_ok=True)
            lib_file.write_text(content, encoding="utf-8")
        else:
            if verbose:
                print(f"Copying: {src_file} -> {lib_file}")
            lib_file.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src_file, lib_file)


def migrate_package(
    src_dir: pathlib.Path, lib_dir: pathlib.Path, mapping_path: pathlib.Path
) -> None:
    """Validate the complete migration before writing or removing legacy files.

    The JSON map names legacy files beside lib_dir, mapping every marked class
    to its canonical file relative to src_dir. Empty maps retire unmarked files.
    No test modules are imported or executed.
    """
    from update_lib.patch_spec import (
        _iter_class_methods,
        _iter_module_classes,
        apply_patches,
        build_patch_dict,
        extract_doctest_guards,
        iter_patches,
    )

    mapping = json.loads(mapping_path.read_text(encoding="utf-8"))
    if not isinstance(mapping, dict) or not mapping:
        raise ValueError("Legacy mapping must be a nonempty object")
    if lib_dir.exists():
        raise ValueError(f"Destination already exists: {lib_dir}")
    if (
        src_dir.resolve() == lib_dir.resolve()
        or src_dir.resolve() in lib_dir.resolve().parents
    ):
        raise ValueError("Source and destination overlap")
    source_files = sorted(
        p
        for p in src_dir.rglob("*")
        if p.is_file() and "__pycache__" not in p.relative_to(src_dir).parts[:-1]
    )
    if not (src_dir / "__init__.py").is_file():
        raise ValueError("Source must be a Python package")
    if any(
        p.is_symlink() or src_dir.resolve() not in p.resolve().parents
        for p in source_files
    ):
        raise ValueError("Source files must remain inside the canonical package")
    contents = {p.relative_to(src_dir): p.read_bytes() for p in source_files}
    entries = collections.defaultdict(list)
    owners = {}
    legacy_files = []
    for name, class_map in mapping.items():
        if pathlib.Path(name).name != name or not name.endswith(".py"):
            raise ValueError(f"Legacy file must be a sibling .py file: {name}")
        if not isinstance(class_map, dict):
            raise ValueError(f"Class map must be an object: {name}")
        legacy = lib_dir.parent / name
        if legacy.is_symlink() or legacy.resolve().parent != lib_dir.parent.resolve():
            raise ValueError(f"Legacy file escapes destination parent: {name}")
        if (
            legacy.resolve() == lib_dir.resolve()
            or src_dir.resolve() in legacy.resolve().parents
        ):
            raise ValueError(f"Legacy file overlaps canonical package: {name}")
        legacy_files.append(legacy)
        legacy_content = legacy.read_text(encoding="utf-8")
        if extract_doctest_guards(legacy_content):
            raise ValueError("Legacy package migration cannot select doctest guards")
        patches = list(iter_patches(legacy_content))
        legacy_classes = collections.defaultdict(list)
        for cls in _iter_module_classes(ast.parse(legacy_content)):
            legacy_classes[cls.name].append(cls)
        if set(class_map) != {p.parent_class for p in patches}:
            raise ValueError(f"Missing or unused class mapping: {name}")
        for entry in patches:
            source_classes = legacy_classes[entry.parent_class]
            if len(source_classes) != 1:
                raise ValueError(f"Ambiguous legacy class: {entry.parent_class}")
            if (
                entry.test_name != "__self__"
                and sum(
                    method.name == entry.test_name
                    for method in _iter_class_methods(source_classes[0])
                )
                != 1
            ):
                raise ValueError(
                    f"Ambiguous legacy method: {entry.parent_class}.{entry.test_name}"
                )
            target = pathlib.Path(class_map[entry.parent_class])
            if target.is_absolute() or ".." in target.parts or target not in contents:
                raise ValueError(f"Invalid canonical destination: {target}")
            if target.suffix != ".py":
                raise ValueError(f"Marker destination is not Python: {target}")
            tree = ast.parse(contents[target].decode("utf-8"))
            classes = collections.defaultdict(list)
            for cls in _iter_module_classes(tree):
                classes[cls.name].append(cls)

            def has_method(class_name, seen):
                if class_name in seen:
                    return False
                seen = seen | {class_name}
                nodes = classes.get(class_name, [])
                if len(nodes) != 1:
                    raise ValueError(f"Missing or ambiguous class: {class_name}")
                cls = nodes[0]
                methods = [
                    m for m in _iter_class_methods(cls) if m.name == entry.test_name
                ]
                if len(methods) > 1:
                    raise ValueError(
                        f"Ambiguous method: {class_name}.{entry.test_name}"
                    )
                if methods or entry.test_name == "__self__":
                    return True
                return any(
                    has_method(base.id, seen)
                    for base in cls.bases
                    if isinstance(base, ast.Name) and base.id in classes
                )

            if not has_method(entry.parent_class, set()):
                raise ValueError(
                    f"Missing method: {entry.parent_class}.{entry.test_name}"
                )
            owner_key = (target, entry.parent_class, entry.test_name)
            if owner_key in owners and owners[owner_key] != legacy:
                raise ValueError(
                    f"Duplicate mapped marker: {entry.parent_class}.{entry.test_name}"
                )
            owners[owner_key] = legacy
            entries[target].append(entry)
    # Render and validate everything before changing the destination.
    for target, patches in entries.items():
        contents[target] = apply_patches(
            contents[target].decode("utf-8"), build_patch_dict(iter(patches))
        ).encode("utf-8")
    for target, data in contents.items():
        if target.suffix == ".py":
            text = data.decode("utf-8")
            ast.parse(text)
            if collections.Counter(iter_patches(text)) != collections.Counter(
                entries[target]
            ):
                raise ValueError(f"Marker roundtrip mismatch: {target}")
    for target, data in contents.items():
        destination = lib_dir / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    for legacy in legacy_files:
        legacy.unlink()


def remove_canonical_files(src_dir, manifest_path, *, dry_run=False):
    """Remove only explicitly reviewed files, validating the whole batch first."""

    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"Duplicate removal metadata key: {key}")
            result[key] = value
        return result

    def relative(name):
        path = pathlib.PurePosixPath(name) if isinstance(name, str) else None
        if (
            not path
            or not path.parts
            or path.is_absolute()
            or ".." in path.parts
            or "\\" in name
            or path.as_posix() != name
            or "\0" in name
        ):
            raise ValueError("Expected an exact relative POSIX filename")
        return pathlib.Path(path)

    def no_symlinks(path):
        if any(p.is_symlink() or p.is_junction() for p in (path, *path.parents)):
            raise ValueError(f"Symlink or junction in removal path: {path}")

    def git(*args):
        result = subprocess.run(
            ["git", "-C", str(src_dir), *args],
            env={
                **os.environ,
                "GIT_NO_LAZY_FETCH": "1",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_TERMINAL_PROMPT": "0",
            },
            capture_output=True,
            check=False,
        )
        if result.returncode:
            raise ValueError("Cannot verify pinned canonical source")
        return result.stdout.decode().strip()

    document = json.loads(
        manifest_path.read_text(encoding="utf-8"), object_pairs_hook=unique_object
    )
    if not isinstance(document, dict) or set(document) != {"source", "removals"}:
        raise ValueError("Expected source and removals metadata")
    source, removals = document["source"], document["removals"]
    if not isinstance(source, dict) or set(source) != {"path", "commit", "tree"}:
        raise ValueError("Expected source path, commit and tree")
    package = relative(source["path"])
    if (
        len(package.parts) != 3
        or package.parts[:2] != ("Lib", "test")
        or not package.name.startswith("test_")
        or not package.name.isidentifier()
    ):
        raise ValueError("Removals require an exact Lib/test/test_* package")
    if any(
        not isinstance(source[k], str) or not re.fullmatch(r"[0-9a-f]{40}", source[k])
        for k in ("commit", "tree")
    ):
        raise ValueError("Source commit and tree must be full Git hashes")
    if not isinstance(removals, dict) or not removals:
        raise ValueError("Removals must be a nonempty filename map")
    src_dir = src_dir.absolute()
    destination = pathlib.Path.cwd() / package
    for path in (src_dir, destination):
        no_symlinks(path)
        if not path.is_dir():
            raise ValueError("Source and destination packages must exist")
    if (
        src_dir == destination
        or src_dir in destination.parents
        or destination in src_dir.parents
    ):
        raise ValueError("Source and destination overlap")
    if (
        src_dir != pathlib.Path(git("rev-parse", "--show-toplevel")) / package
        or git("rev-parse", "HEAD") != source["commit"]
        or git("rev-parse", f"HEAD:{package.as_posix()}") != source["tree"]
    ):
        raise ValueError("Canonical source identity differs from the pinned input")
    canonical = set(
        git(
            "ls-tree",
            "--full-tree",
            "-r",
            "--name-only",
            "-z",
            "HEAD",
            "--",
            package.as_posix(),
        ).split("\0")
    )
    snapshots, receipt = {}, {"source": source, "dry_run": dry_run, "removals": {}}
    for name, entry in sorted(removals.items()):
        path = relative(name)
        if (
            not isinstance(entry, dict)
            or set(entry) != {"sha256", "upstream_commit"}
            or any(
                not isinstance(entry[k], str)
                or not re.fullmatch(f"[0-9a-f]{{{length}}}", entry[k])
                for k, length in (("sha256", 64), ("upstream_commit", 40))
            )
        ):
            raise ValueError(f"Missing or invalid hash/provenance pins: {name}")
        target = destination / path
        no_symlinks(target)
        no_symlinks(src_dir / path)
        if (package / path).as_posix() in canonical or (src_dir / path).exists():
            raise ValueError(f"Selected file is still canonical: {name}")
        if target.exists() and not target.is_file():
            raise ValueError(f"Removal target is not a regular file: {name}")
        data = target.read_bytes() if target.exists() else None
        if data is not None and (
            hashlib.sha256(data).hexdigest() != entry["sha256"]
            or b"rustpython" in data.lower()
        ):
            raise ValueError(f"Changed bytes or RustPython customization: {name}")
        snapshots[target] = data
        receipt["removals"][name] = {
            **entry,
            "status": "already_absent"
            if data is None
            else "would_remove"
            if dry_run
            else "removed",
        }
    if git("rev-parse", "HEAD") != source["commit"]:
        raise ValueError("Canonical source changed during validation")
    for target, data in snapshots.items():
        no_symlinks(target)
        canonical_path = src_dir / target.relative_to(destination)
        no_symlinks(canonical_path)
        if canonical_path.exists():
            raise ValueError("Canonical source changed during validation")
        if (target.exists() and not target.is_file()) or (
            target.read_bytes() if target.exists() else None
        ) != data:
            raise ValueError(f"Removal target changed during validation: {target}")
    if not dry_run:
        for target, data in snapshots.items():
            if data is not None:
                target.unlink()
    print(json.dumps(receipt, sort_keys=True))
    return receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "path",
        type=pathlib.Path,
        help="Source path containing /Lib/ (file or directory)",
    )

    parser.add_argument(
        "--legacy-map",
        type=pathlib.Path,
        help="Explicit legacy-file/class to package-file JSON mapping",
    )
    parser.add_argument(
        "--canonical-removals",
        type=pathlib.Path,
        help="Remove only selected unchanged historical files absent from a pinned canonical tree",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate canonical removals without deleting",
    )

    parser.add_argument(
        "--patches",
        type=pathlib.Path,
        help="Single-file target, remaining decorator patches, and optional doctests JSON",
    )
    parser.add_argument(
        "--support-assignments",
        nargs="+",
        help="Add selected canonical test.support assignments",
    )
    parser.add_argument(
        "--support-python-symlink",
        action="store_true",
        help="Adapt the canonical Windows PythonSymlink library environment",
    )
    parser.add_argument("--support-exports", nargs="+", default=[])
    parser.add_argument("--source-sha256")
    parser.add_argument("--destination-sha256")
    args = parser.parse_args(argv)

    try:
        if args.canonical_removals is not None:
            if (
                args.legacy_map is not None
                or args.patches is not None
                or args.support_assignments
                or args.support_python_symlink
                or args.support_exports
                or args.source_sha256
                or args.destination_sha256
            ):
                raise ValueError(
                    "Canonical removals cannot combine with other migration modes"
                )
            try:
                remove_canonical_files(
                    args.path, args.canonical_removals, dry_run=args.dry_run
                )
            except (OSError, UnicodeError) as exc:
                raise ValueError(str(exc)) from exc
            return 0
        if args.dry_run:
            raise ValueError("--dry-run requires --canonical-removals")
        if args.support_python_symlink:
            if (
                args.legacy_map is not None
                or args.patches is not None
                or args.support_assignments
                or args.support_exports
            ):
                raise ValueError(
                    "PythonSymlink migration cannot combine with other migration modes"
                )
            migrate_support_python_symlink(
                args.path, args.source_sha256, args.destination_sha256
            )
            return 0
        if args.support_assignments:
            if args.legacy_map is not None or args.patches is not None:
                raise ValueError(
                    "Selective support imports cannot combine with --legacy-map or --patches"
                )
            try:
                migrate_support_assignments(
                    args.path,
                    args.support_assignments,
                    args.support_exports,
                    args.source_sha256,
                    args.destination_sha256,
                )
            except (SyntaxError, UnicodeError) as exc:
                raise ValueError(str(exc)) from exc
            return 0
        if args.support_exports or args.source_sha256 or args.destination_sha256:
            raise ValueError("Selective options require --support-assignments")
        if args.patches is not None and (
            args.path.is_dir() or args.legacy_map is not None
        ):
            raise ValueError("--patches requires a single file without --legacy-map")
        document = (
            json.loads(args.patches.read_text()) if args.patches is not None else None
        )
        if args.legacy_map is not None:
            migrate_package(args.path, parse_lib_path(args.path), args.legacy_map)
        elif args.path.is_dir():
            patch_directory(args.path)
        else:
            patch_file(args.path, patches_document=document)
        return 0
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
