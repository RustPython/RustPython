#!/usr/bin/env python
"""
Show dependency information for a module.

Usage:
    python scripts/update_lib deps dis
    python scripts/update_lib deps dataclasses
"""

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))


def show_deps(name: str, cpython_prefix: str = "cpython") -> None:
    """Show all dependency information for a module."""
    from update_lib.deps import (
        DEPENDENCIES,
        get_lib_paths,
        get_soft_deps,
        get_test_paths,
    )

    print(f"Module: {name}")

    # lib paths
    lib_paths = get_lib_paths(name, cpython_prefix)
    for p in lib_paths:
        exists = "+" if p.exists() else "-"
        print(f"  [{exists}] lib: {p}")

    # test paths
    test_paths = get_test_paths(name, cpython_prefix)
    for p in test_paths:
        exists = "+" if p.exists() else "-"
        print(f"  [{exists}] test: {p}")

    # hard_deps (from DEPENDENCIES table)
    dep_info = DEPENDENCIES.get(name, {})
    hard_deps = dep_info.get("hard_deps", [])
    if hard_deps:
        print(f"  hard_deps: {hard_deps}")

    # soft_deps (auto-detected)
    soft_deps = sorted(get_soft_deps(name, cpython_prefix))
    if soft_deps:
        print(f"  soft_deps: {soft_deps}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "name",
        help="Module name (e.g., dis, dataclasses, datetime)",
    )
    parser.add_argument(
        "--cpython",
        default="cpython",
        help="CPython directory prefix (default: cpython)",
    )

    args = parser.parse_args(argv)

    try:
        show_deps(args.name, args.cpython)
        return 0
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
