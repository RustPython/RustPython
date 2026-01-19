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
        Lib/dataclasses.py -> Lib/test/test_dataclasses/
    """
    path_str = str(src_path).replace("\\", "/")
    lib_marker = "/Lib/"

    if lib_marker in path_str:
        lib_path = parse_lib_path(src_path)
        lib_name = lib_path.stem if lib_path.suffix == ".py" else lib_path.name
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
