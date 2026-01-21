#!/usr/bin/env python
"""
Show dependency information for a module.

Usage:
    python scripts/update_lib deps dis
    python scripts/update_lib deps dataclasses
    python scripts/update_lib deps dis --depth 2
    python scripts/update_lib deps all          # Show all modules' dependencies
"""

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))


def get_all_modules(cpython_prefix: str = "cpython") -> list[str]:
    """Get all top-level module names from cpython/Lib/.

    Returns:
        Sorted list of module names (without .py extension)
    """
    lib_dir = pathlib.Path(cpython_prefix) / "Lib"
    if not lib_dir.exists():
        return []

    modules = set()
    for entry in lib_dir.iterdir():
        # Skip private/internal modules and special directories
        if entry.name.startswith(("_", ".")):
            continue
        # Skip test directory
        if entry.name == "test":
            continue

        if entry.is_file() and entry.suffix == ".py":
            modules.add(entry.stem)
        elif entry.is_dir() and (entry / "__init__.py").exists():
            modules.add(entry.name)

    return sorted(modules)


def format_deps_tree(
    cpython_prefix: str,
    lib_prefix: str,
    max_depth: int,
    *,
    name: str | None = None,
    soft_deps: set[str] | None = None,
    _depth: int = 0,
    _visited: set[str] | None = None,
    _indent: str = "",
) -> list[str]:
    """Format soft dependencies as a tree with up-to-date status.

    Args:
        cpython_prefix: CPython directory prefix
        lib_prefix: Local Lib directory prefix
        max_depth: Maximum recursion depth
        name: Module name (used to compute deps if soft_deps not provided)
        soft_deps: Pre-computed soft dependencies (optional)
        _depth: Current depth (internal)
        _visited: Already visited modules (internal)
        _indent: Current indentation (internal)

    Returns:
        List of formatted lines
    """
    from update_lib.deps import (
        get_rust_deps,
        get_soft_deps,
        is_up_to_date,
    )

    lines = []

    if _visited is None:
        _visited = set()

    # Compute deps from name if not provided
    if soft_deps is None:
        soft_deps = get_soft_deps(name, cpython_prefix) if name else set()

    soft_deps = sorted(soft_deps)

    if not soft_deps:
        return lines

    # Separate up-to-date and outdated modules
    up_to_date_deps = []
    outdated_deps = []
    dup_deps = []

    for dep in soft_deps:
        up_to_date = is_up_to_date(dep, cpython_prefix, lib_prefix)
        if up_to_date:
            # Up-to-date modules collected compactly, no dup tracking needed
            up_to_date_deps.append(dep)
        elif dep in _visited:
            # Only track dup for outdated modules
            dup_deps.append(dep)
        else:
            outdated_deps.append(dep)

    # Show outdated modules with expansion
    for dep in outdated_deps:
        dep_native = get_rust_deps(dep, cpython_prefix)
        native_suffix = (
            f" (native: {', '.join(sorted(dep_native))})" if dep_native else ""
        )
        lines.append(f"{_indent}- [ ] {dep}{native_suffix}")
        _visited.add(dep)

        # Recurse if within depth limit
        if _depth < max_depth - 1:
            lines.extend(
                format_deps_tree(
                    cpython_prefix,
                    lib_prefix,
                    max_depth,
                    name=dep,
                    _depth=_depth + 1,
                    _visited=_visited,
                    _indent=_indent + "  ",
                )
            )

    # Show duplicates compactly (only for outdated)
    if dup_deps:
        lines.append(f"{_indent}- [ ] {', '.join(dup_deps)}")

    # Show up-to-date modules compactly on one line
    if up_to_date_deps:
        lines.append(f"{_indent}- [x] {', '.join(up_to_date_deps)}")

    return lines


def format_deps(
    name: str,
    cpython_prefix: str = "cpython",
    lib_prefix: str = "Lib",
    max_depth: int = 10,
    _visited: set[str] | None = None,
) -> list[str]:
    """Format all dependency information for a module.

    Args:
        name: Module name
        cpython_prefix: CPython directory prefix
        lib_prefix: Local Lib directory prefix
        max_depth: Maximum recursion depth
        _visited: Shared visited set for deduplication across modules

    Returns:
        List of formatted lines
    """
    from update_lib.deps import (
        DEPENDENCIES,
        get_lib_paths,
        get_test_paths,
    )

    if _visited is None:
        _visited = set()

    lines = []

    # lib paths (only show existing)
    lib_paths = get_lib_paths(name, cpython_prefix)
    for p in lib_paths:
        if p.exists():
            lines.append(f"[+] lib: {p}")

    # test paths (only show existing)
    test_paths = get_test_paths(name, cpython_prefix)
    for p in test_paths:
        if p.exists():
            lines.append(f"[+] test: {p}")

    # hard_deps (from DEPENDENCIES table)
    dep_info = DEPENDENCIES.get(name, {})
    hard_deps = dep_info.get("hard_deps", [])
    if hard_deps:
        lines.append(f"hard_deps: {hard_deps}")

    lines.append("soft_deps:")
    lines.extend(
        format_deps_tree(
            cpython_prefix, lib_prefix, max_depth, soft_deps={name}, _visited=_visited
        )
    )

    return lines


def show_deps(
    names: list[str],
    cpython_prefix: str = "cpython",
    lib_prefix: str = "Lib",
    max_depth: int = 10,
) -> None:
    """Show all dependency information for modules."""
    # Expand "all" to all module names
    expanded_names = []
    for name in names:
        if name == "all":
            expanded_names.extend(get_all_modules(cpython_prefix))
        else:
            expanded_names.append(name)

    # Shared visited set across all modules
    visited: set[str] = set()

    for i, name in enumerate(expanded_names):
        if i > 0:
            print()  # blank line between modules
        for line in format_deps(name, cpython_prefix, lib_prefix, max_depth, visited):
            print(line)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "names",
        nargs="+",
        help="Module names (e.g., dis, dataclasses) or 'all' for all modules",
    )
    parser.add_argument(
        "--cpython",
        default="cpython",
        help="CPython directory prefix (default: cpython)",
    )
    parser.add_argument(
        "--lib",
        default="Lib",
        help="Local Lib directory prefix (default: Lib)",
    )
    parser.add_argument(
        "--depth",
        type=int,
        default=10,
        help="Maximum recursion depth for soft_deps tree (default: 10)",
    )

    args = parser.parse_args(argv)

    try:
        show_deps(args.names, args.cpython, args.lib, args.depth)
        return 0
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
