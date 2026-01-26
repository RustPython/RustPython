#!/usr/bin/env python
"""
Update library tools for RustPython.

Usage:
    python scripts/update_lib quick cpython/Lib/test/test_foo.py
    python scripts/update_lib copy-lib cpython/Lib/dataclasses.py
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py
    python scripts/update_lib patches --from Lib/test/foo.py --to cpython/Lib/test/foo.py
    python scripts/update_lib auto-mark Lib/test/test_foo.py
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Update library tools for RustPython",
    )

    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser(
        "quick",
        help="Quick update: patch + auto-mark (recommended)",
        add_help=False,
    )
    subparsers.add_parser(
        "migrate",
        help="Migrate test file(s) from CPython, preserving RustPython markers",
        add_help=False,
    )
    subparsers.add_parser(
        "patches",
        help="Patch management (extract/apply patches between files)",
        add_help=False,
    )
    subparsers.add_parser(
        "auto-mark",
        help="Run tests and auto-mark failures with @expectedFailure",
        add_help=False,
    )
    subparsers.add_parser(
        "copy-lib",
        help="Copy library file/directory from CPython (delete existing first)",
        add_help=False,
    )
    subparsers.add_parser(
        "deps",
        help="Show dependency information for a module",
        add_help=False,
    )
    subparsers.add_parser(
        "todo",
        help="Show prioritized list of modules to update",
        add_help=False,
    )

    args, remaining = parser.parse_known_args(argv)

    if args.command == "quick":
        from update_lib.cmd_quick import main as quick_main

        return quick_main(remaining)

    if args.command == "copy-lib":
        from update_lib.cmd_copy_lib import main as copy_lib_main

        return copy_lib_main(remaining)

    if args.command == "migrate":
        from update_lib.cmd_migrate import main as migrate_main

        return migrate_main(remaining)

    if args.command == "patches":
        from update_lib.cmd_patches import main as patches_main

        return patches_main(remaining)

    if args.command == "auto-mark":
        from update_lib.cmd_auto_mark import main as cmd_auto_mark_main

        return cmd_auto_mark_main(remaining)

    if args.command == "deps":
        from update_lib.cmd_deps import main as cmd_deps_main

        return cmd_deps_main(remaining)

    if args.command == "todo":
        from update_lib.cmd_todo import main as cmd_todo_main

        return cmd_todo_main(remaining)

    return 0


if __name__ == "__main__":
    sys.exit(main())
