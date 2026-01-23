#!/usr/bin/env python
"""
Show prioritized list of modules to update.

Usage:
    python scripts/update_lib todo
    python scripts/update_lib todo --limit 20
"""

import argparse
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))


def compute_todo_list(
    cpython_prefix: str = "cpython",
    lib_prefix: str = "Lib",
    include_done: bool = False,
) -> list[dict]:
    """Compute prioritized list of modules to update.

    Scoring:
        - Modules with no pylib dependencies: score = -1
        - Modules with pylib dependencies: score = count of NOT up-to-date deps

    Sorting (ascending by score):
        1. More reverse dependencies (modules depending on this) = higher priority
        2. Fewer native dependencies = higher priority

    Returns:
        List of dicts with module info, sorted by priority
    """
    from update_lib.deps import get_rust_deps, get_soft_deps, is_up_to_date
    from update_lib.show_deps import get_all_modules

    all_modules = get_all_modules(cpython_prefix)

    # Build dependency data for all modules
    module_data = {}
    for name in all_modules:
        soft_deps = get_soft_deps(name, cpython_prefix)
        native_deps = get_rust_deps(name, cpython_prefix)
        up_to_date = is_up_to_date(name, cpython_prefix, lib_prefix)

        module_data[name] = {
            "name": name,
            "soft_deps": soft_deps,
            "native_deps": native_deps,
            "up_to_date": up_to_date,
        }

    # Build reverse dependency map: who depends on this module
    reverse_deps: dict[str, set[str]] = {name: set() for name in all_modules}
    for name, data in module_data.items():
        for dep in data["soft_deps"]:
            if dep in reverse_deps:
                reverse_deps[dep].add(name)

    # Compute scores and filter
    result = []
    for name, data in module_data.items():
        # Skip already up-to-date modules (unless --done)
        if data["up_to_date"] and not include_done:
            continue

        soft_deps = data["soft_deps"]
        if not soft_deps:
            # No pylib dependencies
            score = -1
            total_deps = 0
        else:
            # Count NOT up-to-date dependencies
            score = sum(
                1
                for dep in soft_deps
                if dep in module_data and not module_data[dep]["up_to_date"]
            )
            total_deps = len(soft_deps)

        result.append(
            {
                "name": name,
                "score": score,
                "total_deps": total_deps,
                "reverse_deps": reverse_deps[name],
                "reverse_deps_count": len(reverse_deps[name]),
                "native_deps_count": len(data["native_deps"]),
                "native_deps": data["native_deps"],
                "soft_deps": soft_deps,
                "up_to_date": data["up_to_date"],
            }
        )

    # Sort by:
    # 1. score (ascending) - fewer outstanding deps first
    # 2. reverse_deps_count (descending) - more dependents first
    # 3. native_deps_count (ascending) - fewer native deps first
    result.sort(
        key=lambda x: (
            x["score"],
            -x["reverse_deps_count"],
            x["native_deps_count"],
        )
    )

    return result


def format_todo_list(
    todo_list: list[dict],
    limit: int | None = None,
    verbose: bool = False,
) -> list[str]:
    """Format todo list for display.

    Args:
        todo_list: List from compute_todo_list()
        limit: Maximum number of items to show
        verbose: Show detailed dependency information

    Returns:
        List of formatted lines
    """
    lines = []

    if limit:
        todo_list = todo_list[:limit]

    for item in todo_list:
        name = item["name"]
        score = item["score"]
        total_deps = item["total_deps"]
        rev_count = item["reverse_deps_count"]

        done_mark = "[x]" if item["up_to_date"] else "[ ]"

        if score == -1:
            score_str = "no deps"
        else:
            score_str = f"{score}/{total_deps} deps"

        rev_str = f"{rev_count} dependents" if rev_count else ""

        parts = ["-", done_mark, f"[{score_str}]", name]
        if rev_str:
            parts.append(f"({rev_str})")

        lines.append(" ".join(parts))

        # Verbose mode: show detailed dependency info
        if verbose:
            if item["reverse_deps"]:
                lines.append(f"  dependents: {', '.join(sorted(item['reverse_deps']))}")
            if item["soft_deps"]:
                lines.append(f"  python: {', '.join(sorted(item['soft_deps']))}")
            if item["native_deps"]:
                lines.append(f"  native: {', '.join(sorted(item['native_deps']))}")

    return lines


def show_todo(
    cpython_prefix: str = "cpython",
    lib_prefix: str = "Lib",
    limit: int | None = None,
    include_done: bool = False,
    verbose: bool = False,
) -> None:
    """Show prioritized list of modules to update."""
    todo_list = compute_todo_list(cpython_prefix, lib_prefix, include_done)
    for line in format_todo_list(todo_list, limit, verbose):
        print(line)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
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
        "--limit",
        type=int,
        default=None,
        help="Maximum number of items to show",
    )
    parser.add_argument(
        "--done",
        action="store_true",
        help="Include already up-to-date modules",
    )
    parser.add_argument(
        "--verbose",
        "-v",
        action="store_true",
        help="Show detailed dependency information",
    )

    args = parser.parse_args(argv)

    try:
        show_todo(args.cpython, args.lib, args.limit, args.done, args.verbose)
        return 0
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
