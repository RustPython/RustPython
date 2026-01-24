#!/usr/bin/env python
"""
Quick update for test files from CPython.

Usage:
    # Library + test: copy lib, then patch + auto-mark test + commit
    python scripts/update_lib quick cpython/Lib/dataclasses.py

    # Shortcut: just the module name
    python scripts/update_lib quick dataclasses

    # Test file: patch + auto-mark
    python scripts/update_lib quick cpython/Lib/test/test_foo.py

    # Test file: migrate only
    python scripts/update_lib quick cpython/Lib/test/test_foo.py --no-auto-mark

    # Test file: auto-mark only (Lib/ path implies --no-migrate)
    python scripts/update_lib quick Lib/test/test_foo.py

    # Directory: patch all + auto-mark all
    python scripts/update_lib quick cpython/Lib/test/test_dataclasses/

    # Skip git commit
    python scripts/update_lib quick dataclasses --no-commit
"""

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from update_lib.deps import get_test_paths
from update_lib.io_utils import safe_read_text
from update_lib.path import (
    construct_lib_path,
    get_module_name,
    get_test_files,
    is_lib_path,
    is_test_path,
    lib_to_test_path,
    parse_lib_path,
    resolve_module_path,
)


def collect_original_methods(
    lib_path: pathlib.Path,
) -> set[tuple[str, str]] | dict[pathlib.Path, set[tuple[str, str]]] | None:
    """
    Collect original test methods from lib path before patching.

    Returns:
        - For file: set of (class_name, method_name) or None if file doesn't exist
        - For directory: dict mapping file path to set of methods, or None if dir doesn't exist
    """
    from update_lib.auto_mark import extract_test_methods

    if not lib_path.exists():
        return None

    if lib_path.is_file():
        content = safe_read_text(lib_path)
        return extract_test_methods(content) if content else set()
    else:
        result = {}
        for lib_file in get_test_files(lib_path):
            content = safe_read_text(lib_file)
            if content:
                result[lib_file.resolve()] = extract_test_methods(content)
        return result


def quick(
    src_path: pathlib.Path,
    no_migrate: bool = False,
    no_auto_mark: bool = False,
    mark_failure: bool = False,
    verbose: bool = True,
    skip_build: bool = False,
) -> None:
    """
    Process a file or directory: migrate + auto-mark.

    Args:
        src_path: Source path (file or directory)
        no_migrate: Skip migration step
        no_auto_mark: Skip auto-mark step
        mark_failure: Add @expectedFailure to ALL failing tests
        verbose: Print progress messages
        skip_build: Skip cargo build, use pre-built binary
    """
    from update_lib.auto_mark import auto_mark_directory, auto_mark_file
    from update_lib.migrate import patch_directory, patch_file

    # Determine lib_path and whether to migrate
    if is_lib_path(src_path):
        no_migrate = True
        lib_path = src_path
    else:
        lib_path = parse_lib_path(src_path)

    is_dir = src_path.is_dir()

    # Capture original test methods before migration (for smart auto-mark)
    original_methods = collect_original_methods(lib_path)

    # Step 1: Migrate
    if not no_migrate:
        if is_dir:
            patch_directory(src_path, lib_path, verbose=verbose)
        else:
            patch_file(src_path, lib_path, verbose=verbose)

        # Step 1.5: Handle test dependencies
        from update_lib.deps import get_test_dependencies

        test_deps = get_test_dependencies(src_path)

        # Migrate dependency files
        for dep_src in test_deps["hard_deps"]:
            dep_lib = parse_lib_path(dep_src)
            if verbose:
                print(f"Migrating dependency: {dep_src.name}")
            if dep_src.is_dir():
                patch_directory(dep_src, dep_lib, verbose=False)
            else:
                patch_file(dep_src, dep_lib, verbose=False)

        # Copy data directories (no migration)
        import shutil

        for data_src in test_deps["data"]:
            data_lib = parse_lib_path(data_src)
            if verbose:
                print(f"Copying data: {data_src.name}")
            if data_lib.exists():
                if data_lib.is_dir():
                    shutil.rmtree(data_lib)
                else:
                    data_lib.unlink()
            if data_src.is_dir():
                shutil.copytree(data_src, data_lib)
            else:
                data_lib.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(data_src, data_lib)

    # Step 2: Auto-mark
    if not no_auto_mark:
        if not lib_path.exists():
            raise FileNotFoundError(f"Path not found: {lib_path}")

        if is_dir:
            num_added, num_removed, _ = auto_mark_directory(
                lib_path,
                mark_failure=mark_failure,
                verbose=verbose,
                original_methods_per_file=original_methods,
                skip_build=skip_build,
            )
        else:
            num_added, num_removed, _ = auto_mark_file(
                lib_path,
                mark_failure=mark_failure,
                verbose=verbose,
                original_methods=original_methods,
                skip_build=skip_build,
            )

        if verbose:
            if num_added:
                print(f"Added expectedFailure to {num_added} tests")
            print(f"Removed expectedFailure from {num_removed} tests")


def get_cpython_dir(src_path: pathlib.Path) -> pathlib.Path:
    """Extract cpython directory from source path.

    Example:
        cpython/Lib/dataclasses.py -> cpython
        /some/path/cpython/Lib/foo.py -> /some/path/cpython
    """
    path_str = str(src_path).replace("\\", "/")
    lib_marker = "/Lib/"
    if lib_marker in path_str:
        idx = path_str.index(lib_marker)
        return pathlib.Path(path_str[:idx])
    # Shortcut case: assume "cpython"
    return pathlib.Path("cpython")


def get_cpython_version(cpython_dir: pathlib.Path) -> str:
    """Get CPython version from git tag."""
    import subprocess

    result = subprocess.run(
        ["git", "describe", "--tags"],
        cwd=cpython_dir,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def git_commit(
    name: str,
    lib_path: pathlib.Path | None,
    test_paths: list[pathlib.Path] | pathlib.Path | None,
    cpython_dir: pathlib.Path,
    verbose: bool = True,
) -> bool:
    """Commit changes with CPython author.

    Args:
        name: Module name (e.g., "dataclasses")
        lib_path: Path to library file/directory (or None)
        test_paths: Path(s) to test file/directory (or None)
        cpython_dir: Path to cpython directory
        verbose: Print progress messages

    Returns:
        True if commit was created, False otherwise
    """
    import subprocess

    # Normalize test_paths to list
    if test_paths is None:
        test_paths = []
    elif isinstance(test_paths, pathlib.Path):
        test_paths = [test_paths]

    # Stage changes
    paths_to_add = []
    if lib_path and lib_path.exists():
        paths_to_add.append(str(lib_path))
    for test_path in test_paths:
        if test_path and test_path.exists():
            paths_to_add.append(str(test_path))

    if not paths_to_add:
        return False

    version = get_cpython_version(cpython_dir)
    subprocess.run(["git", "add"] + paths_to_add, check=True)

    # Check if there are staged changes
    result = subprocess.run(
        ["git", "diff", "--cached", "--quiet"],
        capture_output=True,
    )
    if result.returncode == 0:
        if verbose:
            print("No changes to commit")
        return False

    # Commit with CPython author
    message = f"Update {name} from {version}"
    subprocess.run(
        [
            "git",
            "commit",
            "--author",
            "CPython Developers <>",
            "-m",
            message,
        ],
        check=True,
    )
    if verbose:
        print(f"Committed: {message}")
    return True


def _expand_shortcut(path: pathlib.Path) -> pathlib.Path:
    """Expand simple name to cpython/Lib path if it exists.

    Examples:
        dataclasses -> cpython/Lib/dataclasses.py (if exists)
        json -> cpython/Lib/json/ (if exists)
        test_types -> cpython/Lib/test/test_types.py (if exists)
        regrtest -> cpython/Lib/test/libregrtest (from DEPENDENCIES)
    """
    # Only expand if it's a simple name (no path separators) and doesn't exist
    if "/" in str(path) or path.exists():
        return path

    name = str(path)

    # Check DEPENDENCIES table for path overrides (e.g., regrtest)
    from update_lib.deps import DEPENDENCIES

    if name in DEPENDENCIES and "lib" in DEPENDENCIES[name]:
        lib_paths = DEPENDENCIES[name]["lib"]
        if lib_paths:
            override_path = construct_lib_path("cpython", lib_paths[0])
            if override_path.exists():
                return override_path

    # Test shortcut: test_foo -> cpython/Lib/test/test_foo
    if name.startswith("test_"):
        resolved = resolve_module_path(f"test/{name}", "cpython", prefer="dir")
        if resolved.exists():
            return resolved

    # Library shortcut: foo -> cpython/Lib/foo
    resolved = resolve_module_path(name, "cpython", prefer="file")
    if resolved.exists():
        return resolved

    # Extension module shortcut: winreg -> cpython/Lib/test/test_winreg
    # For C/Rust extension modules that have no Python source but have tests
    resolved = resolve_module_path(f"test/test_{name}", "cpython", prefer="dir")
    if resolved.exists():
        return resolved

    # Return original (will likely fail later with a clear error)
    return path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "path",
        type=pathlib.Path,
        help="Source path (file or directory)",
    )
    parser.add_argument(
        "--copy",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Copy library file (default: enabled, implied disabled if test path)",
    )
    parser.add_argument(
        "--migrate",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Migrate test file (default: enabled, implied disabled if Lib/ path)",
    )
    parser.add_argument(
        "--auto-mark",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Auto-mark test failures (default: enabled)",
    )
    parser.add_argument(
        "--mark-failure",
        action="store_true",
        help="Add @expectedFailure to failing tests",
    )
    parser.add_argument(
        "--commit",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Create git commit (default: enabled)",
    )
    parser.add_argument(
        "--build",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Build with cargo (default: enabled)",
    )

    args = parser.parse_args(argv)

    try:
        src_path = args.path

        # Shortcut: expand simple name to cpython/Lib path
        src_path = _expand_shortcut(src_path)
        original_src = src_path  # Keep for commit

        # Track library path for commit
        lib_file_path = None
        test_path = None

        # If it's a library path (not test path), do copy_lib first
        if not is_test_path(src_path):
            # Get library destination path for commit
            lib_file_path = parse_lib_path(src_path)

            if args.copy:
                from update_lib.copy_lib import copy_lib

                copy_lib(src_path)

            # Get all test paths from DEPENDENCIES (or fall back to default)
            module_name = get_module_name(original_src)
            cpython_dir = get_cpython_dir(original_src)
            test_src_paths = get_test_paths(module_name, str(cpython_dir))

            # Fall back to default test path if DEPENDENCIES has no entry
            if not test_src_paths:
                default_test = lib_to_test_path(original_src)
                if default_test.exists():
                    test_src_paths = (default_test,)

            # Process all test paths
            test_paths_for_commit = []
            for test_src in test_src_paths:
                if not test_src.exists():
                    print(f"Warning: Test path does not exist: {test_src}")
                    continue

                test_lib_path = parse_lib_path(test_src)
                test_paths_for_commit.append(test_lib_path)

                quick(
                    test_src,
                    no_migrate=not args.migrate,
                    no_auto_mark=not args.auto_mark,
                    mark_failure=args.mark_failure,
                    skip_build=not args.build,
                )

            test_paths = test_paths_for_commit
        else:
            # It's a test path - process single test
            test_path = (
                parse_lib_path(src_path) if not is_lib_path(src_path) else src_path
            )

            quick(
                src_path,
                no_migrate=not args.migrate,
                no_auto_mark=not args.auto_mark,
                mark_failure=args.mark_failure,
                skip_build=not args.build,
            )
            test_paths = [test_path]

        # Step 3: Git commit
        if args.commit:
            cpython_dir = get_cpython_dir(original_src)
            git_commit(
                get_module_name(original_src), lib_file_path, test_paths, cpython_dir
            )

        return 0
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except Exception as e:
        # Handle TestRunError with a clean message
        from update_lib.auto_mark import TestRunError

        if isinstance(e, TestRunError):
            print(f"Error: {e}", file=sys.stderr)
            return 1
        raise


if __name__ == "__main__":
    sys.exit(main())
