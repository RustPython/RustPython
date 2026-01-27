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

from update_lib.deps import (
    count_test_todos,
    is_test_tracked,
    is_test_up_to_date,
)


def compute_todo_list(
    cpython_prefix: str,
    lib_prefix: str,
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
    from update_lib.cmd_deps import get_all_modules
    from update_lib.deps import (
        get_all_hard_deps,
        get_rust_deps,
        get_soft_deps,
        is_up_to_date,
    )

    all_modules = get_all_modules(cpython_prefix)

    # Build dependency data for all modules
    module_data = {}
    for name in all_modules:
        soft_deps = get_soft_deps(name, cpython_prefix)
        native_deps = get_rust_deps(name, cpython_prefix)
        up_to_date = is_up_to_date(name, cpython_prefix, lib_prefix)

        # Get hard_deps and check their status
        hard_deps = get_all_hard_deps(name, cpython_prefix)
        hard_deps_status = {
            hd: is_up_to_date(hd, cpython_prefix, lib_prefix) for hd in hard_deps
        }

        module_data[name] = {
            "name": name,
            "soft_deps": soft_deps,
            "native_deps": native_deps,
            "up_to_date": up_to_date,
            "hard_deps_status": hard_deps_status,
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
        hard_deps_status = data["hard_deps_status"]
        has_outdated_hard_deps = any(not ok for ok in hard_deps_status.values())

        # Include if: not up-to-date, or has outdated hard_deps, or --done
        if data["up_to_date"] and not has_outdated_hard_deps and not include_done:
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
                "hard_deps_status": hard_deps_status,
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


def get_all_tests(cpython_prefix: str) -> list[str]:
    """Get all test module names from cpython/Lib/test/.

    Returns:
        Sorted list of test names (e.g., ["test_abc", "test_dis", ...])
    """
    test_dir = pathlib.Path(cpython_prefix) / "Lib" / "test"
    if not test_dir.exists():
        return []

    tests = set()
    for entry in test_dir.iterdir():
        # Skip private/internal and special directories
        if entry.name.startswith(("_", ".")):
            continue
        # Skip non-test items
        if not entry.name.startswith("test_"):
            continue

        if entry.is_file() and entry.suffix == ".py":
            tests.add(entry.stem)
        elif entry.is_dir() and (entry / "__init__.py").exists():
            tests.add(entry.name)

    return sorted(tests)


def get_untracked_files(
    cpython_prefix: str,
    lib_prefix: str,
) -> list[str]:
    """Get files that exist in cpython/Lib but not in our Lib.

    Excludes files that belong to tracked modules (shown in library todo)
    and hard_deps of those modules.
    Includes all file types (.py, .txt, .pem, .json, etc.)

    Returns:
        Sorted list of relative paths (e.g., ["foo.py", "data/file.txt"])
    """
    from update_lib.cmd_deps import get_all_modules
    from update_lib.deps import resolve_hard_dep_parent

    cpython_lib = pathlib.Path(cpython_prefix) / "Lib"
    local_lib = pathlib.Path(lib_prefix)

    if not cpython_lib.exists():
        return []

    # Get tracked modules (shown in library todo)
    tracked_modules = set(get_all_modules(cpython_prefix))

    untracked = []

    for cpython_file in cpython_lib.rglob("*"):
        # Skip directories
        if cpython_file.is_dir():
            continue

        # Get relative path from Lib/
        rel_path = cpython_file.relative_to(cpython_lib)

        # Skip test/ directory (handled separately by test todo)
        if rel_path.parts and rel_path.parts[0] == "test":
            continue

        # Check if file belongs to a tracked module
        # e.g., idlelib/Icons/idle.gif -> module "idlelib"
        # e.g., foo.py -> module "foo"
        first_part = rel_path.parts[0]
        if first_part.endswith(".py"):
            module_name = first_part[:-3]  # Remove .py
        else:
            module_name = first_part

        if module_name in tracked_modules:
            continue

        # Check if this is a hard_dep of a tracked module
        if resolve_hard_dep_parent(module_name, cpython_prefix) is not None:
            continue

        # Check if exists in local lib
        local_file = local_lib / rel_path
        if not local_file.exists():
            untracked.append(str(rel_path))

    return sorted(untracked)


def get_original_files(
    cpython_prefix: str,
    lib_prefix: str,
) -> list[str]:
    """Get top-level files/modules that exist in our Lib but not in cpython/Lib.

    These are RustPython-original files that don't come from CPython.
    Modules that exist in cpython are handled by the library todo (even if
    they have additional local files), so they are excluded here.
    Excludes test/ directory (handled separately).

    Returns:
        Sorted list of top-level names (e.g., ["_dummy_thread.py"])
    """
    cpython_lib = pathlib.Path(cpython_prefix) / "Lib"
    local_lib = pathlib.Path(lib_prefix)

    if not local_lib.exists():
        return []

    original = []

    # Only check top-level entries
    for entry in local_lib.iterdir():
        name = entry.name

        # Skip hidden files and __pycache__
        if name.startswith(".") or name == "__pycache__":
            continue

        # Skip test/ directory (handled separately)
        if name == "test":
            continue

        # Skip site-packages (not a module)
        if name == "site-packages":
            continue

        # Only include if it doesn't exist in cpython at all
        cpython_entry = cpython_lib / name
        if not cpython_entry.exists():
            original.append(name)

    return sorted(original)


def _build_test_to_lib_map(
    cpython_prefix: str,
) -> tuple[dict[str, str], dict[str, list[str]]]:
    """Build reverse mapping from test name to library name using DEPENDENCIES.

    Returns:
        Tuple of:
        - Dict mapping test_name -> lib_name (e.g., "test_htmlparser" -> "html")
        - Dict mapping lib_name -> ordered list of test_names
    """
    import pathlib

    from update_lib.deps import DEPENDENCIES

    test_to_lib = {}
    lib_test_order: dict[str, list[str]] = {}
    for lib_name, dep_info in DEPENDENCIES.items():
        if "test" not in dep_info:
            continue
        lib_test_order[lib_name] = []
        for test_path in dep_info["test"]:
            # test_path is like "test_htmlparser.py" or "test_multiprocessing_fork"
            path = pathlib.Path(test_path)
            if path.suffix == ".py":
                test_name = path.stem
            else:
                test_name = path.name
            test_to_lib[test_name] = lib_name
            lib_test_order[lib_name].append(test_name)

    return test_to_lib, lib_test_order


def compute_test_todo_list(
    cpython_prefix: str,
    lib_prefix: str,
    include_done: bool = False,
    lib_status: dict[str, bool] | None = None,
) -> list[dict]:
    """Compute prioritized list of tests to update.

    Scoring:
        - If corresponding lib is up-to-date: score = 0 (ready)
        - If no corresponding lib: score = 1 (independent)
        - If corresponding lib is NOT up-to-date: score = 2 (wait for lib)

    Returns:
        List of dicts with test info, sorted by priority
    """
    all_tests = get_all_tests(cpython_prefix)
    test_to_lib, lib_test_order = _build_test_to_lib_map(cpython_prefix)

    result = []
    for test_name in all_tests:
        up_to_date = is_test_up_to_date(test_name, cpython_prefix, lib_prefix)

        if up_to_date and not include_done:
            continue

        tracked = is_test_tracked(test_name, cpython_prefix, lib_prefix)

        # Check DEPENDENCIES mapping first, then fall back to simple extraction
        if test_name in test_to_lib:
            lib_name = test_to_lib[test_name]
            # Get order from DEPENDENCIES
            test_order = lib_test_order[lib_name].index(test_name)
        else:
            # Extract lib name from test name (test_foo -> foo)
            lib_name = test_name.removeprefix("test_")
            test_order = 0  # Default order for tests not in DEPENDENCIES

        # Check if corresponding lib is up-to-date
        # Scoring: 0 = lib ready (highest priority), 1 = no lib, 2 = lib pending
        if lib_status and lib_name in lib_status:
            lib_up_to_date = lib_status[lib_name]
            if lib_up_to_date:
                score = 0  # Lib is ready, can update test
            else:
                score = 2  # Wait for lib first
        else:
            score = 1  # No corresponding lib (independent test)

        todo_count = count_test_todos(test_name, lib_prefix) if tracked else 0

        result.append(
            {
                "name": test_name,
                "lib_name": lib_name,
                "score": score,
                "up_to_date": up_to_date,
                "tracked": tracked,
                "todo_count": todo_count,
                "test_order": test_order,
            }
        )

    # Sort by score (ascending)
    result.sort(key=lambda x: x["score"])

    return result


def _format_test_suffix(item: dict) -> str:
    """Format suffix for test item (TODO count or untracked)."""
    tracked = item.get("tracked", True)
    if not tracked:
        return " (untracked)"
    todo_count = item.get("todo_count", 0)
    if todo_count > 0:
        return f" ({todo_count} TODO)"
    return ""


def format_test_todo_list(
    todo_list: list[dict],
    limit: int | None = None,
) -> list[str]:
    """Format test todo list for display.

    Groups tests by lib_name. If multiple tests share the same lib_name,
    the first test is shown as the primary and others are indented below it.
    """
    lines = []

    if limit:
        todo_list = todo_list[:limit]

    # Group by lib_name
    grouped: dict[str, list[dict]] = {}
    for item in todo_list:
        lib_name = item.get("lib_name", item["name"])
        if lib_name not in grouped:
            grouped[lib_name] = []
        grouped[lib_name].append(item)

    # Sort each group by test_order (from DEPENDENCIES)
    for tests in grouped.values():
        tests.sort(key=lambda x: x.get("test_order", 0))

    for lib_name, tests in grouped.items():
        # First test is the primary
        primary = tests[0]
        done_mark = "[x]" if primary["up_to_date"] else "[ ]"
        suffix = _format_test_suffix(primary)
        lines.append(f"- {done_mark} {primary['name']}{suffix}")

        # Rest are indented
        for item in tests[1:]:
            done_mark = "[x]" if item["up_to_date"] else "[ ]"
            suffix = _format_test_suffix(item)
            lines.append(f"  - {done_mark} {item['name']}{suffix}")

    return lines


def format_todo_list(
    todo_list: list[dict],
    test_by_lib: dict[str, list[dict]] | None = None,
    limit: int | None = None,
    verbose: bool = False,
) -> list[str]:
    """Format todo list for display.

    Args:
        todo_list: List from compute_todo_list()
        test_by_lib: Dict mapping lib_name -> list of test infos (optional)
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

        parts = ["-", done_mark, f"[{score_str}]", f"`{name}`"]
        if rev_str:
            parts.append(f"({rev_str})")

        lines.append(" ".join(parts))

        # Show hard_deps:
        # - Normal mode: only show if lib is up-to-date but hard_deps are not
        # - Verbose mode: always show all hard_deps with their status
        hard_deps_status = item.get("hard_deps_status", {})
        if verbose and hard_deps_status:
            for hd in sorted(hard_deps_status.keys()):
                hd_mark = "[x]" if hard_deps_status[hd] else "[ ]"
                lines.append(f"  - {hd_mark} {hd} (hard_dep)")
        elif item["up_to_date"]:
            for hd, ok in sorted(hard_deps_status.items()):
                if not ok:
                    lines.append(f"  - [ ] {hd} (hard_dep)")

        # Show corresponding tests if exist
        if test_by_lib and name in test_by_lib:
            for test_info in test_by_lib[name]:
                test_done_mark = "[x]" if test_info["up_to_date"] else "[ ]"
                suffix = _format_test_suffix(test_info)
                lines.append(f"  - {test_done_mark} {test_info['name']}{suffix}")

        # Verbose mode: show detailed dependency info
        if verbose:
            if item["reverse_deps"]:
                lines.append(f"  dependents: {', '.join(sorted(item['reverse_deps']))}")
            if item["soft_deps"]:
                lines.append(f"  python: {', '.join(sorted(item['soft_deps']))}")
            if item["native_deps"]:
                lines.append(f"  native: {', '.join(sorted(item['native_deps']))}")

    return lines


def format_all_todo(
    cpython_prefix: str,
    lib_prefix: str,
    limit: int | None = None,
    include_done: bool = False,
    verbose: bool = False,
) -> list[str]:
    """Format prioritized list of modules and tests to update.

    Returns:
        List of formatted lines
    """
    from update_lib.cmd_deps import get_all_modules
    from update_lib.deps import is_up_to_date

    lines = []

    # Build lib status map for test scoring
    lib_status = {}
    for name in get_all_modules(cpython_prefix):
        lib_status[name] = is_up_to_date(name, cpython_prefix, lib_prefix)

    # Compute test todo (always include all to find libs with pending tests)
    test_todo = compute_test_todo_list(
        cpython_prefix, lib_prefix, include_done=True, lib_status=lib_status
    )

    # Build test_by_lib map (only for tests with corresponding lib)
    test_by_lib: dict[str, list[dict]] = {}
    no_lib_tests = []
    # Set of libs that have pending tests
    libs_with_pending_tests = set()
    for test in test_todo:
        if test["score"] == 1:  # no lib
            if not test["up_to_date"] or include_done:
                no_lib_tests.append(test)
        else:
            lib_name = test["lib_name"]
            if lib_name not in test_by_lib:
                test_by_lib[lib_name] = []
            test_by_lib[lib_name].append(test)
            if not test["up_to_date"]:
                libs_with_pending_tests.add(lib_name)

    # Sort each lib's tests by test_order (from DEPENDENCIES)
    for tests in test_by_lib.values():
        tests.sort(key=lambda x: x.get("test_order", 0))

    # Compute lib todo - include libs with pending tests even if lib is done
    lib_todo_base = compute_todo_list(cpython_prefix, lib_prefix, include_done=True)

    # Filter lib todo: include if lib is not done OR has pending test
    lib_todo = []
    for item in lib_todo_base:
        lib_not_done = not item["up_to_date"]
        has_pending_test = item["name"] in libs_with_pending_tests

        if include_done or lib_not_done or has_pending_test:
            lib_todo.append(item)

    # Format lib todo with embedded tests
    lines.extend(format_todo_list(lib_todo, test_by_lib, limit, verbose))

    # Format "no lib" tests separately if any
    if no_lib_tests:
        lines.append("")
        lines.append("## Standalone Tests")
        lines.extend(format_test_todo_list(no_lib_tests, limit))

    # Format untracked files (in cpython but not in our Lib)
    untracked = get_untracked_files(cpython_prefix, lib_prefix)
    if untracked:
        lines.append("")
        lines.append("## Untracked Files")
        display_untracked = untracked[:limit] if limit else untracked
        for path in display_untracked:
            lines.append(f"- {path}")
        if limit and len(untracked) > limit:
            lines.append(f"  ... and {len(untracked) - limit} more")

    # Format original files (in our Lib but not in cpython)
    original = get_original_files(cpython_prefix, lib_prefix)
    if original:
        lines.append("")
        lines.append("## Original Files")
        display_original = original[:limit] if limit else original
        for path in display_original:
            lines.append(f"- {path}")
        if limit and len(original) > limit:
            lines.append(f"  ... and {len(original) - limit} more")

    return lines


def show_todo(
    cpython_prefix: str,
    lib_prefix: str,
    limit: int | None = None,
    include_done: bool = False,
    verbose: bool = False,
) -> None:
    """Show prioritized list of modules and tests to update."""
    for line in format_all_todo(
        cpython_prefix, lib_prefix, limit, include_done, verbose
    ):
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
