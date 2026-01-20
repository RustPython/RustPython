"""
Path utilities for update_lib.

This module provides functions for:
- Parsing and converting library paths
- Detecting test paths vs library paths
- Extracting test module names from paths
"""

import pathlib


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
        cpython/Lib/dataclasses.py -> cpython/Lib/test/test_dataclasses/ (if dir exists)
        cpython/Lib/typing.py -> cpython/Lib/test/test_typing.py (if file exists)
        cpython/Lib/json/ -> cpython/Lib/test/test_json/
        cpython/Lib/json/__init__.py -> cpython/Lib/test/test_json/
        Lib/dataclasses.py -> Lib/test/test_dataclasses/
    """
    path_str = str(src_path).replace("\\", "/")
    lib_marker = "/Lib/"

    if lib_marker in path_str:
        lib_path = parse_lib_path(src_path)
        lib_name = lib_path.stem if lib_path.suffix == ".py" else lib_path.name
        # Handle __init__.py: use parent directory name
        if lib_name == "__init__":
            lib_name = lib_path.parent.name
        prefix = path_str[: path_str.index(lib_marker)]
        # Try directory first, then file
        dir_path = pathlib.Path(f"{prefix}/Lib/test/test_{lib_name}/")
        if dir_path.exists():
            return dir_path
        file_path = pathlib.Path(f"{prefix}/Lib/test/test_{lib_name}.py")
        if file_path.exists():
            return file_path
        # Default to directory (caller will handle non-existence)
        return dir_path
    else:
        # Path starts with Lib/ - extract name directly
        lib_name = src_path.stem if src_path.suffix == ".py" else src_path.name
        # Handle __init__.py: use parent directory name
        if lib_name == "__init__":
            lib_name = src_path.parent.name
        # Try directory first, then file
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


def test_name_from_path(test_path: pathlib.Path) -> str:
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


# --- Utility functions for reducing duplication ---


def resolve_module_path(
    name: str, prefix: str = "cpython", prefer: str = "file"
) -> pathlib.Path:
    """
    Resolve module path, trying file or directory.

    Args:
        name: Module name (e.g., "dataclasses", "json")
        prefix: CPython directory prefix
        prefer: "file" to try .py first, "dir" to try directory first

    Returns:
        Path to the module (file or directory)

    Examples:
        resolve_module_path("dataclasses") -> cpython/Lib/dataclasses.py
        resolve_module_path("json") -> cpython/Lib/json/
    """
    file_path = pathlib.Path(f"{prefix}/Lib/{name}.py")
    dir_path = pathlib.Path(f"{prefix}/Lib/{name}")

    if prefer == "file":
        if file_path.exists():
            return file_path
        if dir_path.exists():
            return dir_path
        return file_path  # Default to file
    else:
        if dir_path.exists():
            return dir_path
        if file_path.exists():
            return file_path
        return dir_path  # Default to dir


def construct_lib_path(prefix: str, *parts: str) -> pathlib.Path:
    """
    Build a path under prefix/Lib/.

    Args:
        prefix: Directory prefix (e.g., "cpython")
        *parts: Path components after Lib/

    Returns:
        Combined path

    Examples:
        construct_lib_path("cpython", "test", "test_foo.py")
            -> cpython/Lib/test/test_foo.py
        construct_lib_path("cpython", "dataclasses.py")
            -> cpython/Lib/dataclasses.py
    """
    return pathlib.Path(prefix) / "Lib" / pathlib.Path(*parts)


def get_module_name(path: pathlib.Path) -> str:
    """
    Extract module name from path, handling __init__.py.

    Args:
        path: Path to a Python file or directory

    Returns:
        Module name

    Examples:
        get_module_name(Path("cpython/Lib/dataclasses.py")) -> "dataclasses"
        get_module_name(Path("cpython/Lib/json/__init__.py")) -> "json"
        get_module_name(Path("cpython/Lib/json/")) -> "json"
    """
    if path.suffix == ".py":
        name = path.stem
        if name == "__init__":
            return path.parent.name
        return name
    return path.name
