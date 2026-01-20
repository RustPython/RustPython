#!/usr/bin/env python
"""
Copy library files from CPython.

Usage:
    # Single file
    python scripts/update_lib copy-lib cpython/Lib/dataclasses.py

    # Directory
    python scripts/update_lib copy-lib cpython/Lib/json
"""

import argparse
import pathlib
import shutil
import sys


def _copy_single(
    src_path: pathlib.Path,
    lib_path: pathlib.Path,
    verbose: bool = True,
) -> None:
    """Copy a single file or directory."""
    # Remove existing file/directory
    if lib_path.exists():
        if lib_path.is_dir():
            if verbose:
                print(f"Removing directory: {lib_path}")
            shutil.rmtree(lib_path)
        else:
            if verbose:
                print(f"Removing file: {lib_path}")
            lib_path.unlink()

    # Copy
    if src_path.is_dir():
        if verbose:
            print(f"Copying directory: {src_path} -> {lib_path}")
        lib_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(src_path, lib_path)
    else:
        if verbose:
            print(f"Copying file: {src_path} -> {lib_path}")
        lib_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src_path, lib_path)


def copy_lib(
    src_path: pathlib.Path,
    verbose: bool = True,
) -> None:
    """
    Copy library file or directory from CPython.

    Also copies additional files if defined in DEPENDENCIES table.

    Args:
        src_path: Source path (e.g., cpython/Lib/dataclasses.py or cpython/Lib/json)
        verbose: Print progress messages
    """
    from update_lib.deps import get_lib_paths
    from update_lib.path import parse_lib_path

    # Extract module name and cpython prefix from path
    path_str = str(src_path).replace("\\", "/")
    if "/Lib/" in path_str:
        cpython_prefix, after_lib = path_str.split("/Lib/", 1)
        # Get module name (first component, without .py)
        name = after_lib.split("/")[0]
        if name.endswith(".py"):
            name = name[:-3]
    else:
        # Fallback: just copy the single file
        lib_path = parse_lib_path(src_path)
        _copy_single(src_path, lib_path, verbose)
        return

    # Get all paths to copy from DEPENDENCIES table
    all_src_paths = get_lib_paths(name, cpython_prefix)

    # Copy each file
    for src in all_src_paths:
        if src.exists():
            lib_path = parse_lib_path(src)
            _copy_single(src, lib_path, verbose)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "path",
        type=pathlib.Path,
        help="Source path containing /Lib/ (e.g., cpython/Lib/dataclasses.py)",
    )

    args = parser.parse_args(argv)

    try:
        copy_lib(args.path)
        return 0
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
