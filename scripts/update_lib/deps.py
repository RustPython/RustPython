"""
Dependency resolution for library updates.

Handles:
- Irregular library paths (e.g., libregrtest at Lib/test/libregrtest/)
- Library dependencies (e.g., datetime requires _pydatetime)
- Test dependencies (auto-detected from 'from test import ...')
"""

import ast
import pathlib

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


def get_lib_paths(name: str, cpython_prefix: str = "cpython") -> list[pathlib.Path]:
    """Get all library paths for a module.

    Args:
        name: Module name (e.g., "datetime", "libregrtest")
        cpython_prefix: CPython directory prefix

    Returns:
        List of paths to copy
    """
    paths = []
    dep_info = DEPENDENCIES.get(name, {})

    # Get main lib path (override or default)
    if "lib" in dep_info:
        paths = [pathlib.Path(f"{cpython_prefix}/Lib/{p}") for p in dep_info["lib"]]
    else:
        # Default: try file first, then directory
        file_path = pathlib.Path(f"{cpython_prefix}/Lib/{name}.py")
        if file_path.exists():
            paths = [file_path]
        else:
            dir_path = pathlib.Path(f"{cpython_prefix}/Lib/{name}")
            if dir_path.exists():
                paths = [dir_path]
            else:
                paths = [file_path]  # Default to file path

    # Add hard_deps
    if "hard_deps" in dep_info:
        for dep in dep_info["hard_deps"]:
            paths.append(pathlib.Path(f"{cpython_prefix}/Lib/{dep}"))

    return paths


def get_test_paths(name: str, cpython_prefix: str = "cpython") -> list[pathlib.Path]:
    """Get all test paths for a module.

    Args:
        name: Module name (e.g., "datetime", "libregrtest")
        cpython_prefix: CPython directory prefix

    Returns:
        List of test paths
    """
    if name in DEPENDENCIES and "test" in DEPENDENCIES[name]:
        return [
            pathlib.Path(f"{cpython_prefix}/Lib/{p}")
            for p in DEPENDENCIES[name]["test"]
        ]

    # Default: try directory first, then file
    dir_path = pathlib.Path(f"{cpython_prefix}/Lib/test/test_{name}")
    if dir_path.exists():
        return [dir_path]
    file_path = pathlib.Path(f"{cpython_prefix}/Lib/test/test_{name}.py")
    if file_path.exists():
        return [file_path]
    return [dir_path]  # Default to directory path


def get_data_paths(name: str, cpython_prefix: str = "cpython") -> list[pathlib.Path]:
    """Get additional data paths for a module.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix

    Returns:
        List of data paths (may be empty)
    """
    if name in DEPENDENCIES and "data" in DEPENDENCIES[name]:
        return [
            pathlib.Path(f"{cpython_prefix}/Lib/{p}")
            for p in DEPENDENCIES[name]["data"]
        ]
    return []


def parse_test_imports(content: str) -> set[str]:
    """Parse test file content and extract 'from test import ...' dependencies.

    Args:
        content: Python file content

    Returns:
        Set of module names imported from test package
    """
    try:
        tree = ast.parse(content)
    except SyntaxError:
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

    # Collect all test files
    if test_path.is_file():
        files = [test_path]
    else:
        files = list(test_path.glob("**/*.py"))

    # Parse all files for imports (auto-detect deps)
    all_imports = set()
    for f in files:
        try:
            content = f.read_text(encoding="utf-8")
            all_imports.update(parse_test_imports(content))
        except (OSError, UnicodeDecodeError):
            continue

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
        "lib": get_lib_paths(name, cpython_prefix),
        "test": get_test_paths(name, cpython_prefix),
        "data": get_data_paths(name, cpython_prefix),
        "test_deps": [],
    }

    if include_deps:
        # Auto-detect test dependencies
        for test_path in result["test"]:
            deps = get_test_dependencies(test_path)
            for dep in deps:
                if dep not in result["test_deps"]:
                    result["test_deps"].append(dep)

    return result
