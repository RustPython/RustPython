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


def copy_lib(
    src_path: pathlib.Path,
    verbose: bool = True,
) -> None:
    """
    Copy library file or directory from CPython.

    Args:
        src_path: Source path (e.g., cpython/Lib/dataclasses.py or cpython/Lib/json)
        verbose: Print progress messages
    """
    from update_lib.path import parse_lib_path

    lib_path = parse_lib_path(src_path)

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
        shutil.copytree(src_path, lib_path)
    else:
        if verbose:
            print(f"Copying file: {src_path} -> {lib_path}")
        lib_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src_path, lib_path)


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
