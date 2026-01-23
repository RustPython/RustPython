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
        find_dependent_tests_tree,
        get_lib_paths,
        get_test_paths,
        resolve_hard_dep_parent,
    )

    if _visited is None:
        _visited = set()

    lines = []

    # Resolve test_ prefix to module (e.g., test_pydoc -> pydoc)
    if name.startswith("test_"):
        module_name = name[5:]  # strip "test_"
        lines.append(f"(redirecting {name} -> {module_name})")
        name = module_name

    # Resolve hard_dep to parent module (e.g., pydoc_data -> pydoc)
    parent = resolve_hard_dep_parent(name)
    if parent:
        lines.append(f"(redirecting {name} -> {parent})")
        name = parent

    # lib paths (only show existing)
    lib_paths = get_lib_paths(name, cpython_prefix)
    existing_lib_paths = [p for p in lib_paths if p.exists()]
    for p in existing_lib_paths:
        lines.append(f"[+] lib: {p}")

    # test paths (only show existing)
    test_paths = get_test_paths(name, cpython_prefix)
    existing_test_paths = [p for p in test_paths if p.exists()]
    for p in existing_test_paths:
        lines.append(f"[+] test: {p}")

    # If no lib or test paths exist, module doesn't exist
    if not existing_lib_paths and not existing_test_paths:
        lines.append(f"(module '{name}' not found)")
        return lines

    # hard_deps (from DEPENDENCIES table)
    dep_info = DEPENDENCIES.get(name, {})
    hard_deps = dep_info.get("hard_deps", [])
    if hard_deps:
        lines.append(f"packages: {hard_deps}")

    lines.append("\ndependencies:")
    lines.extend(
        format_deps_tree(
            cpython_prefix, lib_prefix, max_depth, soft_deps={name}, _visited=_visited
        )
    )

    # Show dependent tests as tree (depth 2: module + direct importers + their importers)
    tree = find_dependent_tests_tree(name, lib_prefix=lib_prefix, max_depth=2)
    lines.extend(_format_dependent_tests_tree(tree, cpython_prefix, lib_prefix))

    return lines


def _format_dependent_tests_tree(
    tree: dict,
    cpython_prefix: str = "cpython",
    lib_prefix: str = "Lib",
    indent: str = "",
) -> list[str]:
    """Format dependent tests tree for display."""
    from update_lib.deps import is_up_to_date

    lines = []
    module = tree["module"]
    tests = tree["tests"]
    children = tree["children"]

    if indent == "":
        # Root level
        # Count total tests in tree
        def count_tests(t: dict) -> int:
            total = len(t.get("tests", []))
            for c in t.get("children", []):
                total += count_tests(c)
            return total

        total = count_tests(tree)
        if total == 0 and not children:
            lines.append(f"\ndependent tests: (no tests depend on {module})")
            return lines
        lines.append(f"\ndependent tests: ({total} tests)")

    # Check if module is up-to-date
    synced = is_up_to_date(module.split(".")[0], cpython_prefix, lib_prefix)
    marker = "[x]" if synced else "[ ]"

    # Format this node
    if tests:
        test_str = " ".join(tests)
        if indent == "":
            lines.append(f"- {marker} {module}: {test_str}")
        else:
            lines.append(f"{indent}- {marker} {module}: {test_str}")
    elif indent != "" and children:
        # Has children but no direct tests
        lines.append(f"{indent}- {marker} {module}:")

    # Format children
    child_indent = indent + "  " if indent else "    "
    for child in children:
        lines.extend(
            _format_dependent_tests_tree(
                child, cpython_prefix, lib_prefix, child_indent
            )
        )

    return lines


def _resolve_module_name(
    name: str,
    cpython_prefix: str,
    lib_prefix: str,
) -> list[str]:
    """Resolve module name through redirects.

    Returns a list of module names (usually 1, but test support files may expand to multiple).
    """
    import pathlib

    from update_lib.deps import (
        _build_test_import_graph,
        get_lib_paths,
        get_test_paths,
        resolve_hard_dep_parent,
    )

    # Resolve test_ prefix
    if name.startswith("test_"):
        name = name[5:]

    # Resolve hard_dep to parent
    parent = resolve_hard_dep_parent(name)
    if parent:
        return [parent]

    # Check if it's a valid module
    lib_paths = get_lib_paths(name, cpython_prefix)
    test_paths = get_test_paths(name, cpython_prefix)
    if any(p.exists() for p in lib_paths) or any(p.exists() for p in test_paths):
        return [name]

    # Check for test support files (e.g., string_tests -> bytes, str, userstring)
    test_support_path = pathlib.Path(cpython_prefix) / "Lib" / "test" / f"{name}.py"
    if test_support_path.exists():
        test_dir = pathlib.Path(lib_prefix) / "test"
        if test_dir.exists():
            import_graph, _ = _build_test_import_graph(test_dir)
            importing_tests = []
            for file_key, imports in import_graph.items():
                if name in imports and file_key.startswith("test_"):
                    importing_tests.append(file_key)
            if importing_tests:
                # Resolve test names to module names (test_bytes -> bytes)
                return sorted(set(t[5:] for t in importing_tests))

    return [name]


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

    # Resolve and deduplicate names (preserving order)
    seen: set[str] = set()
    resolved_names: list[str] = []
    for name in expanded_names:
        for resolved in _resolve_module_name(name, cpython_prefix, lib_prefix):
            if resolved not in seen:
                seen.add(resolved)
                resolved_names.append(resolved)

    # Shared visited set across all modules
    visited: set[str] = set()

    for i, name in enumerate(resolved_names):
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
