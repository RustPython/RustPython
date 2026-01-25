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
import re
import shelve
import subprocess

from update_lib.file_utils import (
    _dircmp_is_same,
    compare_dir_contents,
    compare_file_contents,
    compare_paths,
    construct_lib_path,
    cpython_to_local_path,
    read_python_files,
    resolve_module_path,
    resolve_test_path,
    safe_parse_ast,
    safe_read_text,
)

# === Import parsing utilities ===


def _extract_top_level_code(content: str) -> str:
    """Extract only top-level code from Python content for faster parsing."""
    def_idx = content.find("\ndef ")
    class_idx = content.find("\nclass ")

    indices = [i for i in (def_idx, class_idx) if i != -1]
    if indices:
        content = content[: min(indices)]
    return content.rstrip("\n")


_FROM_TEST_IMPORT_RE = re.compile(r"^from test import (.+)", re.MULTILINE)
_FROM_TEST_DOT_RE = re.compile(r"^from test\.(\w+)", re.MULTILINE)
_IMPORT_TEST_DOT_RE = re.compile(r"^import test\.(\w+)", re.MULTILINE)


def parse_test_imports(content: str) -> set[str]:
    """Parse test file content and extract test package dependencies."""
    content = _extract_top_level_code(content)
    imports = set()

    for match in _FROM_TEST_IMPORT_RE.finditer(content):
        import_list = match.group(1)
        for part in import_list.split(","):
            name = part.split()[0].strip()
            if name and name not in ("support", "__init__"):
                imports.add(name)

    for match in _FROM_TEST_DOT_RE.finditer(content):
        dep = match.group(1)
        if dep not in ("support", "__init__"):
            imports.add(dep)

    for match in _IMPORT_TEST_DOT_RE.finditer(content):
        dep = match.group(1)
        if dep not in ("support", "__init__"):
            imports.add(dep)

    return imports


_IMPORT_RE = re.compile(r"^import\s+(\w[\w.]*)", re.MULTILINE)
_FROM_IMPORT_RE = re.compile(r"^from\s+(\w[\w.]*)\s+import", re.MULTILINE)


def parse_lib_imports(content: str) -> set[str]:
    """Parse library file and extract all imported module names."""
    imports = set()

    for match in _IMPORT_RE.finditer(content):
        imports.add(match.group(1))

    for match in _FROM_IMPORT_RE.finditer(content):
        imports.add(match.group(1))

    return imports


# === TODO marker utilities ===

TODO_MARKER = "TODO: RUSTPYTHON"


def filter_rustpython_todo(content: str) -> str:
    """Remove lines containing RustPython TODO markers."""
    lines = content.splitlines(keepends=True)
    filtered = [line for line in lines if TODO_MARKER not in line]
    return "".join(filtered)


def count_rustpython_todo(content: str) -> int:
    """Count lines containing RustPython TODO markers."""
    return sum(1 for line in content.splitlines() if TODO_MARKER in line)


def count_todo_in_path(path: pathlib.Path) -> int:
    """Count RustPython TODO markers in a file or directory of .py files."""
    if path.is_file():
        content = safe_read_text(path)
        return count_rustpython_todo(content) if content else 0

    total = 0
    for _, content in read_python_files(path):
        total += count_rustpython_todo(content)
    return total


# === Test utilities ===


def _get_cpython_test_path(test_name: str, cpython_prefix: str) -> pathlib.Path | None:
    """Return the CPython test path for a test name, or None if missing."""
    cpython_path = resolve_test_path(test_name, cpython_prefix, prefer="dir")
    return cpython_path if cpython_path.exists() else None


def _get_local_test_path(
    cpython_test_path: pathlib.Path, lib_prefix: str
) -> pathlib.Path:
    """Return the local Lib/test path matching a CPython test path."""
    return pathlib.Path(lib_prefix) / "test" / cpython_test_path.name


def is_test_tracked(test_name: str, cpython_prefix: str, lib_prefix: str) -> bool:
    """Check if a test exists in the local Lib/test."""
    cpython_path = _get_cpython_test_path(test_name, cpython_prefix)
    if cpython_path is None:
        return True
    local_path = _get_local_test_path(cpython_path, lib_prefix)
    return local_path.exists()


def is_test_up_to_date(test_name: str, cpython_prefix: str, lib_prefix: str) -> bool:
    """Check if a test is up-to-date, ignoring RustPython TODO markers."""
    cpython_path = _get_cpython_test_path(test_name, cpython_prefix)
    if cpython_path is None:
        return True

    local_path = _get_local_test_path(cpython_path, lib_prefix)
    if not local_path.exists():
        return False

    if cpython_path.is_file():
        return compare_file_contents(
            cpython_path, local_path, local_filter=filter_rustpython_todo
        )

    return compare_dir_contents(
        cpython_path, local_path, local_filter=filter_rustpython_todo
    )


def count_test_todos(test_name: str, lib_prefix: str) -> int:
    """Count RustPython TODO markers in a test file/directory."""
    local_dir = pathlib.Path(lib_prefix) / "test" / test_name
    local_file = pathlib.Path(lib_prefix) / "test" / f"{test_name}.py"

    if local_dir.exists():
        return count_todo_in_path(local_dir)
    if local_file.exists():
        return count_todo_in_path(local_file)
    return 0


# === Cross-process cache using shelve ===


def _get_cpython_version(cpython_prefix: str) -> str:
    """Get CPython version from git tag for cache namespace."""
    try:
        result = subprocess.run(
            ["git", "describe", "--tags", "--abbrev=0"],
            cwd=cpython_prefix,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return result.stdout.strip()
    except Exception:
        pass
    return "unknown"


def _get_cache_path() -> str:
    """Get cache file path (without extension - shelve adds its own)."""
    cache_dir = pathlib.Path(__file__).parent / ".cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    return str(cache_dir / "import_graph_cache")


def clear_import_graph_caches() -> None:
    """Clear in-process import graph caches (for testing)."""
    if "_test_import_graph_cache" in globals():
        globals()["_test_import_graph_cache"].clear()
    if "_lib_import_graph_cache" in globals():
        globals()["_lib_import_graph_cache"].clear()


# Manual dependency table for irregular cases
# Format: "name" -> {"lib": [...], "test": [...], "data": [...], "hard_deps": [...]}
# - lib: override default path (default: name.py or name/)
# - hard_deps: additional files to copy alongside the main module
DEPENDENCIES = {
    # regrtest is in Lib/test/libregrtest/, not Lib/libregrtest/
    "regrtest": {
        "lib": ["test/libregrtest"],
        "test": ["test_regrtest"],
        "data": ["test/regrtestdata"],
    },
    # Rust-implemented modules (no lib file, only test)
    "int": {
        "lib": [],
        "hard_deps": ["_pylong.py"],
        "test": [
            "test_int.py",
            "test_long.py",
        ],
    },
    "exception": {
        "lib": [],
        "test": [
            "test_exceptions.py",
            "test_baseexception.py",
            "test_except_star.py",
            "test_exception_group.py",
            "test_exception_hierarchy.py",
            "test_exception_variations.py",
        ],
    },
    "dict": {
        "lib": [],
        "test": [
            "test_dict.py",
            "test_dictcomps.py",
            "test_dictviews.py",
            "test_userdict.py",
        ],
    },
    "list": {
        "lib": [],
        "test": [
            "test_list.py",
            "test_listcomps.py",
            "test_userlist.py",
        ],
    },
    "__future__": {
        "test": [
            "test___future__.py",
            "test_future_stmt.py",
        ],
    },
    "site": {
        "hard_deps": ["_sitebuiltins.py"],
    },
    "opcode": {
        "hard_deps": ["_opcode_metadata.py"],
        "test": [
            "test_opcode.py",
            "test__opcode.py",
            "test_opcodes.py",
        ],
    },
    "pickle": {
        "hard_deps": ["_compat_pickle.py"],
    },
    "re": {
        "hard_deps": ["sre_compile.py", "sre_constants.py", "sre_parse.py"],
    },
    "weakref": {
        "hard_deps": ["_weakrefset.py"],
    },
    "codecs": {
        "test": [
            "test_codecs.py",
            "test_codeccallbacks.py",
            "test_codecencodings_cn.py",
            "test_codecencodings_hk.py",
            "test_codecencodings_iso2022.py",
            "test_codecencodings_jp.py",
            "test_codecencodings_kr.py",
            "test_codecencodings_tw.py",
            "test_codecmaps_cn.py",
            "test_codecmaps_hk.py",
            "test_codecmaps_jp.py",
            "test_codecmaps_kr.py",
            "test_codecmaps_tw.py",
            "test_charmapcodec.py",
            "test_multibytecodec.py",
        ],
    },
    # Non-pattern hard_deps (can't be auto-detected)
    "ast": {
        "hard_deps": ["_ast_unparse.py"],
        "test": [
            "test_ast.py",
            "test_unparse.py",
        ],
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
        "test": [
            "test_support.py",
            "test_script_helper.py",
        ],
    },
    # test_htmlparser tests html.parser
    "html": {
        "hard_deps": ["_markupbase.py"],
        "test": ["test_html.py", "test_htmlparser.py"],
    },
    "xml": {
        "test": [
            "test_xml_etree.py",
            "test_xml_etree_c.py",
            "test_minidom.py",
            "test_pulldom.py",
            "test_pyexpat.py",
            "test_sax.py",
        ],
    },
    "multiprocessing": {
        "test": [
            "test_multiprocessing_fork",
            "test_multiprocessing_forkserver",
            "test_multiprocessing_spawn",
        ],
    },
    "urllib": {
        "test": [
            "test_urllib.py",
            "test_urllib2.py",
            "test_urllib2_localnet.py",
            "test_urllib2net.py",
            "test_urllibnet.py",
            "test_urlparse.py",
            "test_urllib_response.py",
            "test_robotparser.py",
        ],
    },
    "collections": {
        "test": [
            "test_collections.py",
            "test_deque.py",
            "test_defaultdict.py",
            "test_ordered_dict.py",
        ],
    },
    "http": {
        "test": [
            "test_httplib.py",
            "test_http_cookiejar.py",
            "test_http_cookies.py",
            "test_httpservers.py",
        ],
    },
    "unicode": {
        "lib": [],
        "test": [
            "test_unicodedata.py",
            "test_unicode_file.py",
            "test_unicode_file_functions.py",
            "test_unicode_identifiers.py",
            "test_ucn.py",
        ],
    },
    "typing": {
        "test": [
            "test_typing.py",
            "test_type_aliases.py",
            "test_type_annotations.py",
            "test_type_params.py",
            "test_genericalias.py",
        ],
    },
    "unpack": {
        "lib": [],
        "test": [
            "test_unpack.py",
            "test_unpack_ex.py",
        ],
    },
    "zipimport": {
        "test": [
            "test_zipimport.py",
            "test_zipimport_support.py",
        ],
    },
    "time": {
        "lib": [],
        "test": [
            "test_time.py",
            "test_strftime.py",
        ],
    },
    "sys": {
        "lib": [],
        "test": [
            "test_sys.py",
            "test_syslog.py",
            "test_sys_setprofile.py",
            "test_sys_settrace.py",
        ],
    },
    "str": {
        "lib": [],
        "test": [
            "test_str.py",
            "test_fstring.py",
            "test_string_literals.py",
        ],
    },
    "thread": {
        "lib": [],
        "test": [
            "test_thread.py",
            "test_thread_local_bytecode.py",
            "test_threadsignals.py",
        ],
    },
    "threading": {
        "hard_deps": ["_threading_local.py"],
        "test": [
            "test_threading.py",
            "test_threadedtempfile.py",
            "test_threading_local.py",
        ],
    },
    "class": {
        "lib": [],
        "test": [
            "test_class.py",
            "test_genericclass.py",
            "test_subclassinit.py",
        ],
    },
    "generator": {
        "lib": [],
        "test": [
            "test_generators.py",
            "test_genexps.py",
            "test_generator_stop.py",
            "test_yield_from.py",
        ],
    },
    "descr": {
        "lib": [],
        "test": [
            "test_descr.py",
            "test_descrtut.py",
        ],
    },
    "contextlib": {
        "test": [
            "test_contextlib.py",
            "test_contextlib_async.py",
        ],
    },
    "io": {
        "test": [
            "test_io.py",
            "test_bufio.py",
            "test_fileio.py",
            "test_memoryio.py",
        ],
    },
    "dbm": {
        "test": [
            "test_dbm.py",
            "test_dbm_gnu.py",
            "test_dbm_ndbm.py",
        ],
    },
    "datetime": {
        "hard_deps": ["_strptime.py"],
        "test": [
            "test_datetime.py",
            "test_strptime.py",
        ],
    },
    "concurrent": {
        "test": [
            "test_concurrent_futures",
        ],
    },
    "locale": {
        "test": [
            "test_locale.py",
            "test__locale.py",
        ],
    },
    "numbers": {
        "test": [
            "test_numbers.py",
            "test_abstract_numbers.py",
        ],
    },
    "file": {
        "lib": [],
        "test": [
            "test_file.py",
            "test_largefile.py",
        ],
    },
    "fcntl": {
        "lib": [],
        "test": [
            "test_fcntl.py",
            "test_ioctl.py",
        ],
    },
    "select": {
        "lib": [],
        "test": [
            "test_select.py",
            "test_poll.py",
        ],
    },
    "xmlrpc": {
        "test": [
            "test_xmlrpc.py",
            "test_docxmlrpc.py",
        ],
    },
    "ctypes": {
        "test": [
            "test_ctypes",
            "test_stable_abi_ctypes.py",
        ],
    },
}


def resolve_hard_dep_parent(name: str, cpython_prefix: str) -> str | None:
    """Resolve a hard_dep name to its parent module.

    Only returns a parent if the file is actually tracked:
    - Explicitly listed in DEPENDENCIES as a hard_dep
    - Or auto-detected _py{module}.py pattern where the parent module exists

    Args:
        name: Module or file name (with or without .py extension)
        cpython_prefix: CPython directory prefix

    Returns:
        Parent module name if found and tracked, None otherwise
    """
    # Normalize: remove .py extension if present
    if name.endswith(".py"):
        name = name[:-3]

    # Check DEPENDENCIES table first (explicit hard_deps)
    for module_name, dep_info in DEPENDENCIES.items():
        hard_deps = dep_info.get("hard_deps", [])
        for dep in hard_deps:
            # Normalize dep: remove .py extension
            dep_normalized = dep[:-3] if dep.endswith(".py") else dep
            if dep_normalized == name:
                return module_name

    # Auto-detect _py{module} or _py_{module} patterns
    # Only if the parent module actually exists
    if name.startswith("_py"):
        if name.startswith("_py_"):
            # _py_abc -> abc
            parent = name[4:]
        else:
            # _pydatetime -> datetime
            parent = name[3:]

        # Verify the parent module exists
        lib_dir = pathlib.Path(cpython_prefix) / "Lib"
        parent_file = lib_dir / f"{parent}.py"
        parent_dir = lib_dir / parent
        if parent_file.exists() or (
            parent_dir.exists() and (parent_dir / "__init__.py").exists()
        ):
            return parent

    return None


def resolve_test_to_lib(test_name: str) -> str | None:
    """Resolve a test name to its library group from DEPENDENCIES.

    Args:
        test_name: Test name with or without test_ prefix (e.g., "test_urllib2" or "urllib2")

    Returns:
        Library name if test belongs to a group, None otherwise
    """
    # Normalize: add test_ prefix if not present
    if not test_name.startswith("test_"):
        test_name = f"test_{test_name}"

    for lib_name, dep_info in DEPENDENCIES.items():
        tests = dep_info.get("test", [])
        for test_path in tests:
            # test_path is like "test_urllib2.py" or "test_multiprocessing_fork"
            path_stem = test_path[:-3] if test_path.endswith(".py") else test_path
            if path_stem == test_name:
                return lib_name

    return None


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
def get_lib_paths(name: str, cpython_prefix: str) -> tuple[pathlib.Path, ...]:
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

    # Add hard_deps from DEPENDENCIES
    for dep in dep_info.get("hard_deps", []):
        paths.append(construct_lib_path(cpython_prefix, dep))

    # Auto-detect _py{module}.py or _py_{module}.py patterns
    for pattern in [f"_py{name}.py", f"_py_{name}.py"]:
        auto_path = construct_lib_path(cpython_prefix, pattern)
        if auto_path.exists() and auto_path not in paths:
            paths.append(auto_path)

    return tuple(paths)


def get_all_hard_deps(name: str, cpython_prefix: str) -> list[str]:
    """Get all hard_deps for a module (explicit + auto-detected).

    Args:
        name: Module name (e.g., "decimal", "datetime")
        cpython_prefix: CPython directory prefix

    Returns:
        List of hard_dep names (without .py extension)
    """
    dep_info = DEPENDENCIES.get(name, {})
    hard_deps = set()

    # Explicit hard_deps from DEPENDENCIES
    for hd in dep_info.get("hard_deps", []):
        # Remove .py extension if present
        hard_deps.add(hd[:-3] if hd.endswith(".py") else hd)

    # Auto-detect _py{module}.py or _py_{module}.py patterns
    for pattern in [f"_py{name}.py", f"_py_{name}.py"]:
        auto_path = construct_lib_path(cpython_prefix, pattern)
        if auto_path.exists():
            hard_deps.add(auto_path.stem)

    return sorted(hard_deps)


@functools.cache
def get_test_paths(name: str, cpython_prefix: str) -> tuple[pathlib.Path, ...]:
    """Get all test paths for a module.

    Args:
        name: Module name (e.g., "datetime", "libregrtest")
        cpython_prefix: CPython directory prefix

    Returns:
        Tuple of test paths
    """
    if name in DEPENDENCIES and "test" in DEPENDENCIES[name]:
        return tuple(
            construct_lib_path(cpython_prefix, f"test/{p}")
            for p in DEPENDENCIES[name]["test"]
        )

    # Default: try directory first, then file
    return (resolve_module_path(f"test/test_{name}", cpython_prefix, prefer="dir"),)


@functools.cache
def get_all_imports(name: str, cpython_prefix: str) -> frozenset[str]:
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
def get_soft_deps(name: str, cpython_prefix: str) -> frozenset[str]:
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
def get_rust_deps(name: str, cpython_prefix: str) -> frozenset[str]:
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


def is_path_synced(
    cpython_path: pathlib.Path,
    cpython_prefix: str,
    lib_prefix: str,
) -> bool:
    """Check if a CPython path is synced with local.

    Args:
        cpython_path: Path in CPython directory
        cpython_prefix: CPython directory prefix
        lib_prefix: Local Lib directory prefix

    Returns:
        True if synced, False otherwise
    """
    local_path = cpython_to_local_path(cpython_path, cpython_prefix, lib_prefix)
    if local_path is None:
        return False
    return compare_paths(cpython_path, local_path)


@functools.cache
def is_up_to_date(name: str, cpython_prefix: str, lib_prefix: str) -> bool:
    """Check if a module is up-to-date by comparing files.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix
        lib_prefix: Local Lib directory prefix

    Returns:
        True if all files match, False otherwise
    """
    lib_paths = get_lib_paths(name, cpython_prefix)

    found_any = False
    for cpython_path in lib_paths:
        if not cpython_path.exists():
            continue

        found_any = True

        # Convert cpython path to local path
        # cpython/Lib/foo.py -> Lib/foo.py
        rel_path = cpython_path.relative_to(cpython_prefix)
        local_path = pathlib.Path(lib_prefix) / rel_path.relative_to("Lib")

        if not compare_paths(cpython_path, local_path):
            return False

    if not found_any:
        dep_info = DEPENDENCIES.get(name, {})
        if dep_info.get("lib") == []:
            return True
    return found_any


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


_test_import_graph_cache: dict[
    str, tuple[dict[str, set[str]], dict[str, set[str]]]
] = {}


def _is_standard_lib_path(path: str) -> bool:
    """Check if path is the standard Lib directory (not a temp dir)."""
    if "/tmp" in path.lower() or "/var/folders" in path.lower():
        return False
    return (
        path == "Lib/test"
        or path.endswith("/Lib/test")
        or path == "Lib"
        or path.endswith("/Lib")
    )


def _build_test_import_graph(
    test_dir: pathlib.Path,
) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    """Build import graphs for files within test directory (recursive).

    Uses cross-process shelve cache based on CPython version.

    Args:
        test_dir: Path to Lib/test/ directory

    Returns:
        Tuple of:
        - Dict mapping relative path (without .py) -> set of test modules it imports
        - Dict mapping relative path (without .py) -> set of all lib imports
    """
    # In-process cache
    cache_key = str(test_dir)
    if cache_key in _test_import_graph_cache:
        return _test_import_graph_cache[cache_key]

    # Cross-process cache (only for standard Lib/test directory)
    use_file_cache = _is_standard_lib_path(cache_key)
    if use_file_cache:
        version = _get_cpython_version("cpython")
        shelve_key = f"test_import_graph:{version}"
        try:
            with shelve.open(_get_cache_path()) as db:
                if shelve_key in db:
                    import_graph, lib_imports_graph = db[shelve_key]
                    _test_import_graph_cache[cache_key] = (
                        import_graph,
                        lib_imports_graph,
                    )
                    return import_graph, lib_imports_graph
        except Exception:
            pass

    # Build from scratch
    import_graph: dict[str, set[str]] = {}
    lib_imports_graph: dict[str, set[str]] = {}

    for py_file in test_dir.glob("**/*.py"):
        content = safe_read_text(py_file)
        if content is None:
            continue

        imports = set()
        imports.update(parse_test_imports(content))
        all_imports = parse_lib_imports(content)

        for imp in all_imports:
            if (py_file.parent / f"{imp}.py").exists():
                imports.add(imp)
            if (test_dir / f"{imp}.py").exists():
                imports.add(imp)

        submodule_imports = _parse_test_submodule_imports(content)
        for submodule, imported_names in submodule_imports.items():
            submodule_dir = test_dir / submodule
            if submodule_dir.is_dir():
                for name in imported_names:
                    if (submodule_dir / f"{name}.py").exists():
                        imports.add(name)

        rel_path = py_file.relative_to(test_dir)
        key = str(rel_path.with_suffix(""))
        import_graph[key] = imports
        lib_imports_graph[key] = all_imports

    # Save to cross-process cache
    if use_file_cache:
        try:
            with shelve.open(_get_cache_path()) as db:
                db[shelve_key] = (import_graph, lib_imports_graph)
        except Exception:
            pass
    _test_import_graph_cache[cache_key] = (import_graph, lib_imports_graph)

    return import_graph, lib_imports_graph


_lib_import_graph_cache: dict[str, dict[str, set[str]]] = {}


def _build_lib_import_graph(lib_prefix: str) -> dict[str, set[str]]:
    """Build import graph for Lib modules (full module paths like urllib.request).

    Uses cross-process shelve cache based on CPython version.

    Args:
        lib_prefix: RustPython Lib directory

    Returns:
        Dict mapping full_module_path -> set of modules it imports
    """
    # In-process cache
    if lib_prefix in _lib_import_graph_cache:
        return _lib_import_graph_cache[lib_prefix]

    # Cross-process cache (only for standard Lib directory)
    use_file_cache = _is_standard_lib_path(lib_prefix)
    if use_file_cache:
        version = _get_cpython_version("cpython")
        shelve_key = f"lib_import_graph:{version}"
        try:
            with shelve.open(_get_cache_path()) as db:
                if shelve_key in db:
                    import_graph = db[shelve_key]
                    _lib_import_graph_cache[lib_prefix] = import_graph
                    return import_graph
        except Exception:
            pass

    # Build from scratch
    lib_dir = pathlib.Path(lib_prefix)
    if not lib_dir.exists():
        return {}

    import_graph: dict[str, set[str]] = {}

    for entry in lib_dir.iterdir():
        if entry.name.startswith(("_", ".")):
            continue
        if entry.name == "test":
            continue

        if entry.is_file() and entry.suffix == ".py":
            content = safe_read_text(entry)
            if content:
                imports = parse_lib_imports(content)
                imports.discard(entry.stem)
                import_graph[entry.stem] = imports
        elif entry.is_dir() and (entry / "__init__.py").exists():
            for py_file in entry.glob("**/*.py"):
                content = safe_read_text(py_file)
                if content:
                    imports = parse_lib_imports(content)
                    rel_path = py_file.relative_to(lib_dir)
                    if rel_path.name == "__init__.py":
                        full_name = str(rel_path.parent).replace("/", ".")
                    else:
                        full_name = str(rel_path.with_suffix("")).replace("/", ".")
                    imports.discard(full_name.split(".")[0])
                    import_graph[full_name] = imports

    # Save to cross-process cache
    if use_file_cache:
        try:
            with shelve.open(_get_cache_path()) as db:
                db[shelve_key] = import_graph
        except Exception:
            pass
    _lib_import_graph_cache[lib_prefix] = import_graph

    return import_graph


def _get_lib_modules_importing(
    module_name: str, lib_import_graph: dict[str, set[str]]
) -> set[str]:
    """Find Lib modules (full paths) that import module_name or any of its submodules."""
    importers: set[str] = set()
    target_top = module_name.split(".")[0]

    for full_path, imports in lib_import_graph.items():
        if full_path.split(".")[0] == target_top:
            continue  # Skip same package
        # Match if module imports target OR any submodule of target
        # e.g., for "xml": match imports of "xml", "xml.parsers", "xml.etree.ElementTree"
        matches = any(
            imp == module_name or imp.startswith(module_name + ".") for imp in imports
        )
        if matches:
            importers.add(full_path)

    return importers


def _consolidate_submodules(
    modules: set[str], threshold: int = 3
) -> dict[str, set[str]]:
    """Consolidate submodules if count exceeds threshold.

    Args:
        modules: Set of full module paths (e.g., {"urllib.request", "urllib.parse", "xml.dom", "xml.sax"})
        threshold: If submodules > threshold, consolidate to parent

    Returns:
        Dict mapping display_name -> set of original module paths
        e.g., {"urllib.request": {"urllib.request"}, "xml": {"xml.dom", "xml.sax", "xml.etree", "xml.parsers"}}
    """
    # Group by top-level package
    by_package: dict[str, set[str]] = {}
    for mod in modules:
        parts = mod.split(".")
        top = parts[0]
        if top not in by_package:
            by_package[top] = set()
        by_package[top].add(mod)

    result: dict[str, set[str]] = {}
    for top, submods in by_package.items():
        if len(submods) > threshold:
            # Consolidate to top-level
            result[top] = submods
        else:
            # Keep individual
            for mod in submods:
                result[mod] = {mod}

    return result


# Modules that are used everywhere - show but don't expand their dependents
_BLOCKLIST_MODULES = frozenset(
    {
        "unittest",
        "test.support",
        "support",
        "doctest",
        "typing",
        "abc",
        "collections.abc",
        "functools",
        "itertools",
        "operator",
        "contextlib",
        "warnings",
        "types",
        "enum",
        "re",
        "io",
        "os",
        "sys",
    }
)


def find_dependent_tests_tree(
    module_name: str,
    lib_prefix: str,
    max_depth: int = 1,
    _depth: int = 0,
    _visited_tests: set[str] | None = None,
    _visited_modules: set[str] | None = None,
) -> dict:
    """Find dependent tests in a tree structure.

    Args:
        module_name: Module to search for (e.g., "ftplib")
        lib_prefix: RustPython Lib directory
        max_depth: Maximum depth to recurse (default 1 = show direct + 1 level of Lib deps)

    Returns:
        Dict with structure:
        {
            "module": "ftplib",
            "tests": ["test_ftplib", "test_urllib2"],  # Direct importers
            "children": [
                {"module": "urllib.request", "tests": [...], "children": []},
                ...
            ]
        }
    """
    lib_dir = pathlib.Path(lib_prefix)
    test_dir = lib_dir / "test"

    if _visited_tests is None:
        _visited_tests = set()
    if _visited_modules is None:
        _visited_modules = set()

    # Build graphs
    test_import_graph, test_lib_imports = _build_test_import_graph(test_dir)
    lib_import_graph = _build_lib_import_graph(lib_prefix)

    # Find tests that directly import this module
    target_top = module_name.split(".")[0]
    direct_tests: set[str] = set()
    for file_key, imports in test_lib_imports.items():
        if file_key in _visited_tests:
            continue
        # Match exact module OR any child submodule
        # e.g., "xml" matches imports of "xml", "xml.parsers", "xml.etree.ElementTree"
        # but "collections._defaultdict" only matches "collections._defaultdict" (no children)
        matches = any(
            imp == module_name or imp.startswith(module_name + ".") for imp in imports
        )
        if matches:
            # Check if it's a test file
            if pathlib.Path(file_key).name.startswith("test_"):
                direct_tests.add(file_key)
                _visited_tests.add(file_key)

    # Consolidate test names (test_sqlite3/test_dbapi -> test_sqlite3)
    consolidated_tests = {_consolidate_file_key(t) for t in direct_tests}

    # Mark this module as visited (cycle detection)
    _visited_modules.add(module_name)
    _visited_modules.add(target_top)

    children = []
    # Check blocklist and depth limit
    should_expand = (
        _depth < max_depth
        and module_name not in _BLOCKLIST_MODULES
        and target_top not in _BLOCKLIST_MODULES
    )

    if should_expand:
        # Find Lib modules that import this module
        lib_importers = _get_lib_modules_importing(module_name, lib_import_graph)

        # Skip already visited modules (cycle detection) and blocklisted modules
        lib_importers = {
            m
            for m in lib_importers
            if m not in _visited_modules
            and m.split(".")[0] not in _visited_modules
            and m not in _BLOCKLIST_MODULES
            and m.split(".")[0] not in _BLOCKLIST_MODULES
        }

        # Consolidate submodules (xml.dom, xml.sax, xml.etree -> xml if > 3)
        consolidated_libs = _consolidate_submodules(lib_importers, threshold=3)

        # Build children
        for display_name, original_mods in sorted(consolidated_libs.items()):
            child = find_dependent_tests_tree(
                display_name,
                lib_prefix,
                max_depth,
                _depth + 1,
                _visited_tests,
                _visited_modules,
            )
            if child["tests"] or child["children"]:
                children.append(child)

    return {
        "module": module_name,
        "tests": sorted(consolidated_tests),
        "children": children,
    }


def _consolidate_file_key(file_key: str) -> str:
    """Consolidate file_key to test name.

    Args:
        file_key: Relative path without .py (e.g., "test_foo", "test_bar/test_sub")

    Returns:
        Consolidated test name:
        - "test_foo" for "test_foo"
        - "test_sqlite3" for "test_sqlite3/test_dbapi"
    """
    parts = pathlib.Path(file_key).parts
    if len(parts) == 1:
        return parts[0]
    return parts[0]
