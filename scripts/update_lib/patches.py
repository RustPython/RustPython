#!/usr/bin/env python
"""
Patch management for test files.

Usage:
    # Extract patches from one file and apply to another
    python scripts/update_lib patches --from Lib/test/foo.py --to cpython/Lib/test/foo.py

    # Show patches as JSON
    python scripts/update_lib patches --from Lib/test/foo.py --show-patches

    # Apply patches from JSON file
    python scripts/update_lib patches -p patches.json --to Lib/test/foo.py
"""

import argparse
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))


def write_output(data: str, dest: str) -> None:
    if dest == "-":
        print(data, end="")
        return

    with open(dest, "w") as fd:
        fd.write(data)


def main(argv: list[str] | None = None) -> int:
    from update_lib import (
        apply_patches,
        extract_patches,
        patches_from_json,
        patches_to_json,
    )

    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )

    patches_group = parser.add_mutually_exclusive_group(required=True)
    patches_group.add_argument(
        "-p",
        "--patches",
        type=pathlib.Path,
        help="File path to file containing patches in a JSON format",
    )
    patches_group.add_argument(
        "--from",
        dest="gather_from",
        type=pathlib.Path,
        help="File to gather patches from",
    )

    group = parser.add_mutually_exclusive_group(required=False)
    group.add_argument(
        "--to",
        type=pathlib.Path,
        help="File to apply patches to",
    )
    group.add_argument(
        "--show-patches",
        action="store_true",
        help="Show the patches and exit",
    )

    parser.add_argument(
        "-o",
        "--output",
        default="-",
        help="Output file. Set to '-' for stdout",
    )

    args = parser.parse_args(argv)

    # Validate required arguments
    if args.to is None and not args.show_patches:
        parser.error("--to or --show-patches is required")

    try:
        if args.patches:
            patches = patches_from_json(json.loads(args.patches.read_text()))
        else:
            patches = extract_patches(args.gather_from.read_text())

        if args.show_patches:
            output = json.dumps(patches_to_json(patches), indent=4) + "\n"
            write_output(output, args.output)
            return 0

        patched = apply_patches(args.to.read_text(), patches)
        write_output(patched, args.output)
        return 0

    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
