"""
File utilities for update_lib.

This module provides functions for:
- Safe file reading with error handling
- Safe AST parsing with error handling
- Iterating over Python files
- Parsing and converting library paths
- Detecting test paths vs library paths
- Comparing files or directories for equality
"""

from __future__ import annotations

import ast
import filecmp
import pathlib
from collections.abc import Callable, Iterator

# === I/O utilities ===


def safe_read_text(path: pathlib.Path) -> str | None:
    """Read file content with UTF-8 encoding, returning None on error."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


def safe_parse_ast(content: str) -> ast.Module | None:
    """Parse Python content into AST, returning None on syntax error."""
    try:
        return ast.parse(content)
    except SyntaxError:
        return None


def iter_python_files(path: pathlib.Path) -> Iterator[pathlib.Path]:
    """Yield Python files from a file or directory."""
    if path.is_file():
        yield path
    else:
        yield from path.glob("**/*.py")


def read_python_files(path: pathlib.Path) -> Iterator[tuple[pathlib.Path, str]]:
    """Read all Python files from a path, yielding (path, content) pairs."""
    for py_file in iter_python_files(path):
        content = safe_read_text(py_file)
        if content is not None:
            yield py_file, content


# === Path utilities ===


def parse_lib_path(path: pathlib.Path | str) -> pathlib.Path:
    """
    Extract the Lib/... portion from a path containing /Lib/.

    Example:
        parse_lib_path("cpython/Lib/test/foo.py") -> Path("Lib/test/foo.py")
    """
    path_str = str(path).replace("\\", "/")
    lib_marker = "/Lib/"

    if lib_marker not in path_str:
        raise ValueError(f"Path must contain '/Lib/' or '\\Lib\\' (got: {path})")

    idx = path_str.index(lib_marker)
    return pathlib.Path(path_str[idx + 1 :])


def is_lib_path(path: pathlib.Path) -> bool:
    """Check if path starts with Lib/"""
    path_str = str(path).replace("\\", "/")
    return path_str.startswith("Lib/") or path_str.startswith("./Lib/")


def is_test_path(path: pathlib.Path) -> bool:
    """Check if path is a test path (contains /Lib/test/ or starts with Lib/test/)"""
    path_str = str(path).replace("\\", "/")
    return "/Lib/test/" in path_str or path_str.startswith("Lib/test/")


def lib_to_test_path(src_path: pathlib.Path) -> pathlib.Path:
    """
    Convert library path to test path.

    Examples:
        cpython/Lib/dataclasses.py -> cpython/Lib/test/test_dataclasses/
        cpython/Lib/json/__init__.py -> cpython/Lib/test/test_json/
    """
    path_str = str(src_path).replace("\\", "/")
    lib_marker = "/Lib/"

    if lib_marker in path_str:
        lib_path = parse_lib_path(src_path)
        lib_name = lib_path.stem if lib_path.suffix == ".py" else lib_path.name
        if lib_name == "__init__":
            lib_name = lib_path.parent.name
        prefix = path_str[: path_str.index(lib_marker)]
        dir_path = pathlib.Path(f"{prefix}/Lib/test/test_{lib_name}/")
        if dir_path.exists():
            return dir_path
        file_path = pathlib.Path(f"{prefix}/Lib/test/test_{lib_name}.py")
        if file_path.exists():
            return file_path
        return dir_path
    else:
        lib_name = src_path.stem if src_path.suffix == ".py" else src_path.name
        if lib_name == "__init__":
            lib_name = src_path.parent.name
        dir_path = pathlib.Path(f"Lib/test/test_{lib_name}/")
        if dir_path.exists():
            return dir_path
        file_path = pathlib.Path(f"Lib/test/test_{lib_name}.py")
        if file_path.exists():
            return file_path
        return dir_path


def get_test_files(path: pathlib.Path) -> list[pathlib.Path]:
    """Get all .py test files in a path (file or directory)."""
    if path.is_file():
        return [path]
    return sorted(path.glob("**/*.py"))


def get_test_module_name(test_path: pathlib.Path) -> str:
    """
    Extract test module name from a test file path.

    Examples:
        Lib/test/test_foo.py -> test_foo
        Lib/test/test_ctypes/test_bar.py -> test_ctypes.test_bar
    """
    test_path = pathlib.Path(test_path)
    if test_path.parent.name.startswith("test_"):
        return f"{test_path.parent.name}.{test_path.stem}"
    return test_path.stem


def resolve_module_path(
    name: str, prefix: str = "cpython", prefer: str = "file"
) -> pathlib.Path:
    """
    Resolve module path, trying file or directory.

    Args:
        name: Module name (e.g., "dataclasses", "json")
        prefix: CPython directory prefix
        prefer: "file" to try .py first, "dir" to try directory first
    """
    file_path = pathlib.Path(f"{prefix}/Lib/{name}.py")
    dir_path = pathlib.Path(f"{prefix}/Lib/{name}")

    if prefer == "file":
        if file_path.exists():
            return file_path
        if dir_path.exists():
            return dir_path
        return file_path
    else:
        if dir_path.exists():
            return dir_path
        if file_path.exists():
            return file_path
        return dir_path


def construct_lib_path(prefix: str, *parts: str) -> pathlib.Path:
    """Build a path under prefix/Lib/."""
    return pathlib.Path(prefix) / "Lib" / pathlib.Path(*parts)


def resolve_test_path(
    test_name: str, prefix: str = "cpython", prefer: str = "dir"
) -> pathlib.Path:
    """Resolve a test module path under Lib/test/."""
    return resolve_module_path(f"test/{test_name}", prefix, prefer=prefer)


def cpython_to_local_path(
    cpython_path: pathlib.Path,
    cpython_prefix: str,
    lib_prefix: str,
) -> pathlib.Path | None:
    """Convert CPython path to local Lib path."""
    try:
        rel_path = cpython_path.relative_to(cpython_prefix)
        return pathlib.Path(lib_prefix) / rel_path.relative_to("Lib")
    except ValueError:
        return None


def get_module_name(path: pathlib.Path) -> str:
    """Extract module name from path, handling __init__.py."""
    if path.suffix == ".py":
        name = path.stem
        if name == "__init__":
            return path.parent.name
        return name
    return path.name


def get_cpython_dir(src_path: pathlib.Path) -> pathlib.Path:
    """Extract CPython directory from a path containing /Lib/."""
    path_str = str(src_path).replace("\\", "/")
    lib_marker = "/Lib/"
    if lib_marker in path_str:
        idx = path_str.index(lib_marker)
        return pathlib.Path(path_str[:idx])
    return pathlib.Path("cpython")


# === Comparison utilities ===


def _dircmp_is_same(dcmp: filecmp.dircmp) -> bool:
    """Recursively check if two directories are identical."""
    if dcmp.diff_files or dcmp.left_only or dcmp.right_only:
        return False

    for subdir in dcmp.subdirs.values():
        if not _dircmp_is_same(subdir):
            return False

    return True


def compare_paths(cpython_path: pathlib.Path, local_path: pathlib.Path) -> bool:
    """Compare a CPython path with a local path (file or directory)."""
    if not local_path.exists():
        return False

    if cpython_path.is_file():
        return filecmp.cmp(cpython_path, local_path, shallow=False)

    dcmp = filecmp.dircmp(cpython_path, local_path)
    return _dircmp_is_same(dcmp)


def compare_file_contents(
    cpython_path: pathlib.Path,
    local_path: pathlib.Path,
    *,
    local_filter: Callable[[str], str] | None = None,
    encoding: str = "utf-8",
) -> bool:
    """Compare two files as text, optionally filtering local content."""
    try:
        cpython_content = cpython_path.read_text(encoding=encoding)
        local_content = local_path.read_text(encoding=encoding)
    except (OSError, UnicodeDecodeError):
        return False

    if local_filter is not None:
        local_content = local_filter(local_content)

    return cpython_content == local_content


def compare_dir_contents(
    cpython_dir: pathlib.Path,
    local_dir: pathlib.Path,
    *,
    pattern: str = "*.py",
    local_filter: Callable[[str], str] | None = None,
    encoding: str = "utf-8",
) -> bool:
    """Compare directory contents for matching files and text."""
    cpython_files = {f.relative_to(cpython_dir) for f in cpython_dir.rglob(pattern)}
    local_files = {f.relative_to(local_dir) for f in local_dir.rglob(pattern)}

    if cpython_files != local_files:
        return False

    for rel_path in cpython_files:
        if not compare_file_contents(
            cpython_dir / rel_path,
            local_dir / rel_path,
            local_filter=local_filter,
            encoding=encoding,
        ):
            return False

    return True
