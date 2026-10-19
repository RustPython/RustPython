#!/usr/bin/env python
"""
Copy library files from CPython.

Usage:
    # Single file
    python scripts/update_lib copy-lib cpython/Lib/dataclasses.py

    # Directory
    python scripts/update_lib copy-lib cpython/Lib/json

    # Explicit, pinned Cargo-build adaptation (copies only pydoc.py)
    python scripts/update_lib copy-lib cpython/Lib/pydoc.py --pydoc-build-stdlib
"""

import argparse
import pathlib
import shutil
import sys


def _ignore_bytecode_caches(directory: str, names: list[str]) -> set[str]:
    return {
        name
        for name in names
        if name == "__pycache__" and (pathlib.Path(directory) / name).is_dir()
    }


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
        shutil.copytree(src_path, lib_path, ignore=_ignore_bytecode_caches)
    else:
        if verbose:
            print(f"Copying file: {src_path} -> {lib_path}")
        lib_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src_path, lib_path)


def copy_lib(
    src_path: pathlib.Path,
    verbose: bool = True,
    *,
    pydoc_build_stdlib: bool = False,
) -> None:
    """
    Copy library file or directory from CPython.

    Also copies additional files if defined in DEPENDENCIES table.

    Args:
        src_path: Source path (e.g., cpython/Lib/dataclasses.py or cpython/Lib/json)
        verbose: Print progress messages
        pydoc_build_stdlib: Adapt only pinned pydoc.py for Cargo's stdlib layout
    """
    from update_lib.deps import get_lib_paths
    from update_lib.file_utils import parse_lib_path

    # Extract module name and cpython prefix from path
    path_str = str(src_path).replace("\\", "/")
    if "/Lib/" not in path_str:
        raise ValueError(f"Path must contain '/Lib/' (got: {src_path})")
    if not src_path.exists():
        raise FileNotFoundError(f"Source path does not exist: {src_path}")

    if pydoc_build_stdlib:
        from update_lib.lib_transforms import pydoc_build_stdlib as transform

        lib_path = parse_lib_path(src_path)
        if lib_path != pathlib.Path("Lib/pydoc.py") or not src_path.is_file():
            raise ValueError("--pydoc-build-stdlib requires exactly Lib/pydoc.py")
        if src_path.resolve() == lib_path.resolve() or (
            lib_path.exists() and src_path.samefile(lib_path)
        ):
            raise ValueError("pydoc source and destination must be different files")
        if lib_path.is_symlink() or (lib_path.exists() and not lib_path.is_file()):
            raise ValueError("pydoc destination must be a regular file")
        # Validate the complete input before writing anything. This explicit
        # transform must not refresh dependencies or change unrelated files.
        contents = transform(src_path.read_bytes())
        if verbose:
            print(f"Adapting pydoc Cargo stdlib path: {src_path} -> {lib_path}")
        lib_path.parent.mkdir(parents=True, exist_ok=True)
        lib_path.write_bytes(contents)
        shutil.copystat(src_path, lib_path)
        return

    # Data files name an exact asset, not the containing top-level module.
    # Expanding Lib/test/data.txt as "test" would replace the entire test tree.
    if src_path.is_file() and src_path.suffix != ".py":
        _copy_single(src_path, parse_lib_path(src_path), verbose)
        return

    cpython_prefix, after_lib = path_str.split("/Lib/", 1)
    # Get module name (first component, without .py)
    name = after_lib.split("/")[0]
    if name.endswith(".py"):
        name = name[:-3]

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
    parser.add_argument(
        "--pydoc-build-stdlib",
        action="store_true",
        help="Adapt only pinned pydoc.py for RustPython's Cargo-build stdlib path",
    )

    args = parser.parse_args(argv)

    try:
        copy_lib(args.path, pydoc_build_stdlib=args.pydoc_build_stdlib)
        return 0
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
