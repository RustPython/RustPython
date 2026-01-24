#!/usr/bin/env python
"""
Migrate test file(s) from CPython, preserving RustPython markers.

Usage:
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py

This will:
    1. Extract patches from Lib/test/test_foo.py (if exists)
    2. Apply them to cpython/Lib/test/test_foo.py
    3. Write result to Lib/test/test_foo.py
"""

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from update_lib.path import parse_lib_path


def patch_single_content(
    src_path: pathlib.Path,
    lib_path: pathlib.Path,
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

    # Extract patches from existing file (if exists)
    if lib_path.exists():
        patches = extract_patches(lib_path.read_text(encoding="utf-8"))
    else:
        patches = {}

    # Apply patches to source content
    src_content = src_path.read_text(encoding="utf-8")
    return apply_patches(src_content, patches)


def patch_file(
    src_path: pathlib.Path,
    lib_path: pathlib.Path | None = None,
    verbose: bool = True,
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
        content = patch_single_content(src_path, lib_path)
    else:
        if verbose:
            print(f"Copying: {src_path} -> {lib_path}")
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

    src_files = sorted(src_dir.glob("**/*.py"))

    for src_file in src_files:
        rel_path = src_file.relative_to(src_dir)
        lib_file = lib_dir / rel_path

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

    args = parser.parse_args(argv)

    try:
        if args.path.is_dir():
            patch_directory(args.path)
        else:
            patch_file(args.path)
        return 0
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
