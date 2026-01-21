"""
Dependency resolution for library updates.

Handles:
- Irregular library paths (e.g., libregrtest at Lib/test/libregrtest/)
- Library dependencies (e.g., datetime requires _pydatetime)
- Test dependencies (auto-detected from 'from test import ...')
"""

import ast
import functools
import pathlib
from collections import deque

from update_lib.io_utils import read_python_files, safe_parse_ast, safe_read_text
from update_lib.path import construct_lib_path, resolve_module_path

# Manual dependency table for irregular cases
# Format: "name" -> {"lib": [...], "test": [...], "data": [...], "hard_deps": [...]}
# - lib: override default path (default: name.py or name/)
# - hard_deps: additional files to copy alongside the main module
DEPENDENCIES = {
    # regrtest is in Lib/test/libregrtest/, not Lib/libregrtest/
    "regrtest": {
        "lib": ["test/libregrtest"],
        "test": ["test/test_regrtest"],
        "data": ["test/regrtestdata"],
    },
    # Rust-implemented modules (no lib file, only test)
    "int": {
        "lib": [],  # No Python lib (Rust implementation)
        "hard_deps": ["_pylong.py"],
    },
    # Pure Python implementations
    "abc": {
        "hard_deps": ["_py_abc.py"],
    },
    "codecs": {
        "hard_deps": ["_pycodecs.py"],
    },
    "datetime": {
        "hard_deps": ["_pydatetime.py"],
    },
    "decimal": {
        "hard_deps": ["_pydecimal.py"],
    },
    "io": {
        "hard_deps": ["_pyio.py"],
    },
    "warnings": {
        "hard_deps": ["_py_warnings.py"],
    },
    # Data directories
    "pydoc": {
        "hard_deps": ["pydoc_data"],
    },
    "turtle": {
        "hard_deps": ["turtledemo"],
    },
    # Test support library (like regrtest)
    "support": {
        "lib": ["test/support"],
        "data": ["test/wheeldata"],
    },
}

# Test-specific dependencies (only when auto-detection isn't enough)
# - hard_deps: files to migrate (tightly coupled, must be migrated together)
# - data: directories to copy without migration
TEST_DEPENDENCIES = {
    # Audio tests
    "test_winsound": {
        "data": ["audiodata"],
    },
    "test_wave": {
        "data": ["audiodata"],
    },
    "audiotests": {
        "data": ["audiodata"],
    },
    # Archive tests
    "test_tarfile": {
        "data": ["archivetestdata"],
    },
    "test_zipfile": {
        "data": ["archivetestdata"],
    },
    # Config tests
    "test_configparser": {
        "data": ["configdata"],
    },
    "test_config": {
        "data": ["configdata"],
    },
    # Other data directories
    "test_decimal": {
        "data": ["decimaltestdata"],
    },
    "test_dtrace": {
        "data": ["dtracedata"],
    },
    "test_math": {
        "data": ["mathdata"],
    },
    "test_ssl": {
        "data": ["certdata"],
    },
    "test_subprocess": {
        "data": ["subprocessdata"],
    },
    "test_tkinter": {
        "data": ["tkinterdata"],
    },
    "test_tokenize": {
        "data": ["tokenizedata"],
    },
    "test_type_annotations": {
        "data": ["typinganndata"],
    },
    "test_zipimport": {
        "data": ["zipimport_data"],
    },
    # XML tests share xmltestdata
    "test_xml_etree": {
        "data": ["xmltestdata"],
    },
    "test_pulldom": {
        "data": ["xmltestdata"],
    },
    "test_sax": {
        "data": ["xmltestdata"],
    },
    "test_minidom": {
        "data": ["xmltestdata"],
    },
    # Multibytecodec support needs cjkencodings
    "multibytecodec_support": {
        "data": ["cjkencodings"],
    },
    # i18n
    "i18n_helper": {
        "data": ["translationdata"],
    },
    # wheeldata is used by test_makefile and support
    "test_makefile": {
        "data": ["wheeldata"],
    },
}


@functools.cache
def get_lib_paths(
    name: str, cpython_prefix: str = "cpython"
) -> tuple[pathlib.Path, ...]:
    """Get all library paths for a module.

    Args:
        name: Module name (e.g., "datetime", "libregrtest")
        cpython_prefix: CPython directory prefix

    Returns:
        Tuple of paths to copy
    """
    dep_info = DEPENDENCIES.get(name, {})

    # Get main lib path (override or default)
    if "lib" in dep_info:
        paths = [construct_lib_path(cpython_prefix, p) for p in dep_info["lib"]]
    else:
        # Default: try file first, then directory
        paths = [resolve_module_path(name, cpython_prefix, prefer="file")]

    # Add hard_deps
    for dep in dep_info.get("hard_deps", []):
        paths.append(construct_lib_path(cpython_prefix, dep))

    return tuple(paths)


@functools.cache
def get_test_paths(
    name: str, cpython_prefix: str = "cpython"
) -> tuple[pathlib.Path, ...]:
    """Get all test paths for a module.

    Args:
        name: Module name (e.g., "datetime", "libregrtest")
        cpython_prefix: CPython directory prefix

    Returns:
        Tuple of test paths
    """
    if name in DEPENDENCIES and "test" in DEPENDENCIES[name]:
        return tuple(
            construct_lib_path(cpython_prefix, p) for p in DEPENDENCIES[name]["test"]
        )

    # Default: try directory first, then file
    return (resolve_module_path(f"test/test_{name}", cpython_prefix, prefer="dir"),)


@functools.cache
def get_data_paths(
    name: str, cpython_prefix: str = "cpython"
) -> tuple[pathlib.Path, ...]:
    """Get additional data paths for a module.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix

    Returns:
        Tuple of data paths (may be empty)
    """
    if name in DEPENDENCIES and "data" in DEPENDENCIES[name]:
        return tuple(
            construct_lib_path(cpython_prefix, p) for p in DEPENDENCIES[name]["data"]
        )
    return ()


def parse_test_imports(content: str) -> set[str]:
    """Parse test file content and extract 'from test import ...' dependencies.

    Args:
        content: Python file content

    Returns:
        Set of module names imported from test package
    """
    tree = safe_parse_ast(content)
    if tree is None:
        return set()

    imports = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            if node.module == "test":
                # from test import foo, bar
                for alias in node.names:
                    name = alias.name
                    # Skip support modules and common imports
                    if name not in ("support", "__init__"):
                        imports.add(name)
            elif node.module and node.module.startswith("test."):
                # from test.foo import bar -> depends on foo
                parts = node.module.split(".")
                if len(parts) >= 2:
                    dep = parts[1]
                    if dep not in ("support", "__init__"):
                        imports.add(dep)

    return imports


def parse_lib_imports(content: str) -> set[str]:
    """Parse library file and extract all imported module names.

    Args:
        content: Python file content

    Returns:
        Set of imported module names (top-level only)
    """
    tree = safe_parse_ast(content)
    if tree is None:
        return set()

    imports = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            # import foo, bar
            for alias in node.names:
                imports.add(alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            # from foo import bar
            if node.module:
                imports.add(node.module.split(".")[0])

    return imports


@functools.cache
def get_all_imports(name: str, cpython_prefix: str = "cpython") -> frozenset[str]:
    """Get all imports from a library file.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix

    Returns:
        Frozenset of all imported module names
    """
    all_imports = set()
    for lib_path in get_lib_paths(name, cpython_prefix):
        if lib_path.exists():
            for _, content in read_python_files(lib_path):
                all_imports.update(parse_lib_imports(content))

    # Remove self
    all_imports.discard(name)
    return frozenset(all_imports)


@functools.cache
def get_soft_deps(name: str, cpython_prefix: str = "cpython") -> frozenset[str]:
    """Get soft dependencies by parsing imports from library file.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix

    Returns:
        Frozenset of imported stdlib module names (those that exist in cpython/Lib/)
    """
    all_imports = get_all_imports(name, cpython_prefix)

    # Filter: only include modules that exist in cpython/Lib/
    stdlib_deps = set()
    for imp in all_imports:
        module_path = resolve_module_path(imp, cpython_prefix)
        if module_path.exists():
            stdlib_deps.add(imp)

    return frozenset(stdlib_deps)


@functools.cache
def get_rust_deps(name: str, cpython_prefix: str = "cpython") -> frozenset[str]:
    """Get Rust/C dependencies (imports that don't exist in cpython/Lib/).

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix

    Returns:
        Frozenset of imported module names that are built-in or C extensions
    """
    all_imports = get_all_imports(name, cpython_prefix)
    soft_deps = get_soft_deps(name, cpython_prefix)
    return frozenset(all_imports - soft_deps)


def _dircmp_is_same(dcmp) -> bool:
    """Recursively check if two directories are identical.

    Args:
        dcmp: filecmp.dircmp object

    Returns:
        True if directories are identical (including subdirectories)
    """
    if dcmp.diff_files or dcmp.left_only or dcmp.right_only:
        return False

    # Recursively check subdirectories
    for subdir in dcmp.subdirs.values():
        if not _dircmp_is_same(subdir):
            return False

    return True


@functools.cache
def is_up_to_date(
    name: str, cpython_prefix: str = "cpython", lib_prefix: str = "Lib"
) -> bool:
    """Check if a module is up-to-date by comparing files.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix
        lib_prefix: Local Lib directory prefix

    Returns:
        True if all files match, False otherwise
    """
    import filecmp

    lib_paths = get_lib_paths(name, cpython_prefix)

    for cpython_path in lib_paths:
        if not cpython_path.exists():
            continue

        # Convert cpython path to local path
        # cpython/Lib/foo.py -> Lib/foo.py
        rel_path = cpython_path.relative_to(cpython_prefix)
        local_path = pathlib.Path(lib_prefix) / rel_path.relative_to("Lib")

        if not local_path.exists():
            return False

        if cpython_path.is_file():
            if not filecmp.cmp(cpython_path, local_path, shallow=False):
                return False
        else:
            # Directory comparison (recursive)
            dcmp = filecmp.dircmp(cpython_path, local_path)
            if not _dircmp_is_same(dcmp):
                return False

    return True


def get_test_dependencies(
    test_path: pathlib.Path,
) -> dict[str, list[pathlib.Path]]:
    """Get test dependencies by parsing imports.

    Args:
        test_path: Path to test file or directory

    Returns:
        Dict with "hard_deps" (files to migrate) and "data" (dirs to copy)
    """
    result = {"hard_deps": [], "data": []}

    if not test_path.exists():
        return result

    # Parse all files for imports (auto-detect deps)
    all_imports = set()
    for _, content in read_python_files(test_path):
        all_imports.update(parse_test_imports(content))

    # Also add manual dependencies from TEST_DEPENDENCIES
    test_name = test_path.stem if test_path.is_file() else test_path.name
    manual_deps = TEST_DEPENDENCIES.get(test_name, {})
    if "hard_deps" in manual_deps:
        all_imports.update(manual_deps["hard_deps"])

    # Convert imports to paths (deps)
    for imp in all_imports:
        # Check if it's a test file (test_*) or support module
        if imp.startswith("test_"):
            # It's a test, resolve to test path
            dep_path = test_path.parent / f"{imp}.py"
            if not dep_path.exists():
                dep_path = test_path.parent / imp
        else:
            # Support module like string_tests, lock_tests, encoded_modules
            # Check file first, then directory
            dep_path = test_path.parent / f"{imp}.py"
            if not dep_path.exists():
                dep_path = test_path.parent / imp

        if dep_path.exists() and dep_path not in result["hard_deps"]:
            result["hard_deps"].append(dep_path)

    # Add data paths from manual table (for the test file itself)
    if "data" in manual_deps:
        for data_name in manual_deps["data"]:
            data_path = test_path.parent / data_name
            if data_path.exists() and data_path not in result["data"]:
                result["data"].append(data_path)

    # Also add data from auto-detected deps' TEST_DEPENDENCIES
    # e.g., test_codecencodings_kr -> multibytecodec_support -> cjkencodings
    for imp in all_imports:
        dep_info = TEST_DEPENDENCIES.get(imp, {})
        if "data" in dep_info:
            for data_name in dep_info["data"]:
                data_path = test_path.parent / data_name
                if data_path.exists() and data_path not in result["data"]:
                    result["data"].append(data_path)

    return result


def resolve_all_paths(
    name: str,
    cpython_prefix: str = "cpython",
    include_deps: bool = True,
) -> dict[str, list[pathlib.Path]]:
    """Resolve all paths for a module update.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix
        include_deps: Whether to include auto-detected dependencies

    Returns:
        Dict with "lib", "test", "data", "test_deps" keys
    """
    result = {
        "lib": list(get_lib_paths(name, cpython_prefix)),
        "test": list(get_test_paths(name, cpython_prefix)),
        "data": list(get_data_paths(name, cpython_prefix)),
        "test_deps": [],
    }

    if include_deps:
        # Auto-detect test dependencies
        for test_path in result["test"]:
            deps = get_test_dependencies(test_path)
            for dep_path in deps["hard_deps"]:
                if dep_path not in result["test_deps"]:
                    result["test_deps"].append(dep_path)
            for data_path in deps["data"]:
                if data_path not in result["data"]:
                    result["data"].append(data_path)

    return result


@functools.cache
def _build_import_graph(lib_prefix: str = "Lib") -> dict[str, set[str]]:
    """Build a graph of module imports from lib_prefix directory.

    Args:
        lib_prefix: RustPython Lib directory (default: "Lib")

    Returns:
        Dict mapping module_name -> set of modules it imports
    """
    lib_dir = pathlib.Path(lib_prefix)
    if not lib_dir.exists():
        return {}

    import_graph: dict[str, set[str]] = {}

    # Scan all .py files in lib_prefix (excluding test/ directory for module imports)
    for entry in lib_dir.iterdir():
        if entry.name.startswith(("_", ".")):
            continue
        if entry.name == "test":
            continue

        module_name = None
        if entry.is_file() and entry.suffix == ".py":
            module_name = entry.stem
        elif entry.is_dir() and (entry / "__init__.py").exists():
            module_name = entry.name

        if module_name:
            # Parse imports from this module
            imports = set()
            for _, content in read_python_files(entry):
                imports.update(parse_lib_imports(content))
            # Remove self-imports
            imports.discard(module_name)
            import_graph[module_name] = imports

    return import_graph


def _build_reverse_graph(import_graph: dict[str, set[str]]) -> dict[str, set[str]]:
    """Build reverse dependency graph (who imports this module).

    Args:
        import_graph: Forward import graph (module -> imports)

    Returns:
        Reverse graph (module -> imported_by)
    """
    reverse_graph: dict[str, set[str]] = {}

    for module, imports in import_graph.items():
        for imported in imports:
            if imported not in reverse_graph:
                reverse_graph[imported] = set()
            reverse_graph[imported].add(module)

    return reverse_graph


@functools.cache
def get_transitive_imports(
    module_name: str,
    lib_prefix: str = "Lib",
) -> frozenset[str]:
    """Get all modules that transitively depend on module_name.

    Args:
        module_name: Target module
        lib_prefix: RustPython Lib directory (default: "Lib")

    Returns:
        Frozenset of module names that import module_name (directly or indirectly)
    """
    import_graph = _build_import_graph(lib_prefix)
    reverse_graph = _build_reverse_graph(import_graph)

    # BFS from module_name following reverse edges
    visited: set[str] = set()
    queue = deque(reverse_graph.get(module_name, set()))

    while queue:
        current = queue.popleft()
        if current in visited:
            continue
        visited.add(current)
        # Add modules that import current module
        for importer in reverse_graph.get(current, set()):
            if importer not in visited:
                queue.append(importer)

    return frozenset(visited)


def _parse_test_submodule_imports(content: str) -> dict[str, set[str]]:
    """Parse 'from test.X import Y' to get submodule imports.

    Args:
        content: Python file content

    Returns:
        Dict mapping submodule (e.g., "test_bar") -> set of imported names (e.g., {"helper"})
    """
    tree = safe_parse_ast(content)
    if tree is None:
        return {}

    result: dict[str, set[str]] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            if node.module and node.module.startswith("test."):
                # from test.test_bar import helper -> test_bar: {helper}
                parts = node.module.split(".")
                if len(parts) >= 2:
                    submodule = parts[1]
                    if submodule not in ("support", "__init__"):
                        if submodule not in result:
                            result[submodule] = set()
                        for alias in node.names:
                            result[submodule].add(alias.name)

    return result


def _build_test_import_graph(test_dir: pathlib.Path) -> dict[str, set[str]]:
    """Build import graph for files within test directory (recursive).

    Args:
        test_dir: Path to Lib/test/ directory

    Returns:
        Dict mapping relative path (without .py) -> set of test modules it imports
    """
    import_graph: dict[str, set[str]] = {}

    # Use **/*.py to recursively find all Python files
    for py_file in test_dir.glob("**/*.py"):
        content = safe_read_text(py_file)
        if content is None:
            continue

        imports = set()
        # Parse "from test import X" style imports
        imports.update(parse_test_imports(content))
        # Also check direct imports of test modules
        all_imports = parse_lib_imports(content)

        # Check for files at same level or in test_dir
        for imp in all_imports:
            # Check in same directory
            if (py_file.parent / f"{imp}.py").exists():
                imports.add(imp)
            # Check in test_dir root
            if (test_dir / f"{imp}.py").exists():
                imports.add(imp)

        # Handle "from test.X import Y" where Y is a file in test_dir/X/
        submodule_imports = _parse_test_submodule_imports(content)
        for submodule, imported_names in submodule_imports.items():
            submodule_dir = test_dir / submodule
            if submodule_dir.is_dir():
                for name in imported_names:
                    # Check if it's a file in the submodule directory
                    if (submodule_dir / f"{name}.py").exists():
                        imports.add(name)

        # Use relative path from test_dir as key (without .py)
        rel_path = py_file.relative_to(test_dir)
        key = str(rel_path.with_suffix(""))
        import_graph[key] = imports

    return import_graph


@functools.cache
def find_tests_importing_module(
    module_name: str,
    lib_prefix: str = "Lib",
    include_transitive: bool = True,
) -> frozenset[pathlib.Path]:
    """Find all test files that import the given module (directly or transitively).

    Only returns test_*.py files. Support files (like pickletester.py, string_tests.py)
    are used for transitive dependency calculation but not included in the result.

    Args:
        module_name: Module to search for (e.g., "datetime")
        lib_prefix: RustPython Lib directory (default: "Lib")
        include_transitive: Whether to include transitive dependencies

    Returns:
        Frozenset of test_*.py file paths that depend on this module
    """
    lib_dir = pathlib.Path(lib_prefix)
    test_dir = lib_dir / "test"

    if not test_dir.exists():
        return frozenset()

    # Build set of modules to search for (Lib/ modules)
    target_modules = {module_name}
    if include_transitive:
        # Add all modules that transitively depend on module_name
        target_modules.update(get_transitive_imports(module_name, lib_prefix))

    # Build test directory import graph for transitive analysis within test/
    test_import_graph = _build_test_import_graph(test_dir)

    # First pass: find all files (by relative path) that directly import target modules
    directly_importing: set[str] = set()
    for py_file in test_dir.glob("**/*.py"):  # Recursive glob
        content = safe_read_text(py_file)
        if content is None:
            continue
        imports = parse_lib_imports(content)
        if imports & target_modules:
            rel_path = py_file.relative_to(test_dir)
            directly_importing.add(str(rel_path.with_suffix("")))

    # Second pass: find files that transitively import via support files within test/
    # BFS to find all files that import any file in all_importing
    all_importing = set(directly_importing)
    queue = deque(directly_importing)
    while queue:
        current = queue.popleft()
        # Extract the filename (stem) from the relative path for matching
        current_path = pathlib.Path(current)
        current_stem = current_path.name
        # For __init__.py, the import name is the parent directory name
        # e.g., "test_json/__init__" -> can be imported as "test_json"
        if current_stem == "__init__":
            current_stem = current_path.parent.name
        for file_key, imports in test_import_graph.items():
            if current_stem in imports and file_key not in all_importing:
                all_importing.add(file_key)
                queue.append(file_key)

    # Filter to only test_*.py files and build result paths
    result: set[pathlib.Path] = set()
    for file_key in all_importing:
        # file_key is like "test_foo" or "test_bar/test_sub"
        path_parts = pathlib.Path(file_key)
        filename = path_parts.name  # Get just the filename part
        if filename.startswith("test_"):
            result.add(test_dir / f"{file_key}.py")

    return frozenset(result)


def consolidate_test_paths(
    test_paths: frozenset[pathlib.Path],
    test_dir: pathlib.Path,
) -> frozenset[str]:
    """Consolidate test paths by grouping test_*/ directory contents into a single entry.

    Args:
        test_paths: Frozenset of absolute paths to test files
        test_dir: Path to the test directory (e.g., Lib/test)

    Returns:
        Frozenset of consolidated test names:
        - "test_foo" for Lib/test/test_foo.py
        - "test_sqlite3" for any file in Lib/test/test_sqlite3/
    """
    consolidated: set[str] = set()

    for path in test_paths:
        try:
            rel_path = path.relative_to(test_dir)
            parts = rel_path.parts

            if len(parts) == 1:
                # test_foo.py -> test_foo
                consolidated.add(rel_path.stem)
            else:
                # test_sqlite3/test_dbapi.py -> test_sqlite3
                consolidated.add(parts[0])
        except ValueError:
            # Path not relative to test_dir, use stem
            consolidated.add(path.stem)

    return frozenset(consolidated)
