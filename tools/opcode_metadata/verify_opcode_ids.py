"""Verify every active public discriminant against pinned final opcode_ids.h."""

import argparse
import re
from pathlib import Path

from target_source import verify_source

from utils import DEFAULT_INPUT, to_upper_snake_case


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    verify_source(args.source)
    ids = {
        name: int(value)
        for name, value in re.findall(
            r"^#define\s+(\w+)\s+(\d+)\s*$",
            (args.source / "Include/opcode_ids.h").read_text(),
            re.M,
        )
    }
    private = {
        "GetYieldFromIter": 219,
        "LoadConstImmortal": 220,
        "LoadConstMortal": 221,
    }
    entries = re.findall(
        r"^        (\w+)(?:\s*\{[^}]*\})?\s*=\s*(\d+),",
        DEFAULT_INPUT.read_text(),
        re.M,
    )
    assert len(entries) == 238
    assert len({int(value) for _, value in entries}) == len(entries)
    for name, value in entries:
        expected = private[name] if name in private else ids[to_upper_snake_case(name)]
        assert int(value) == expected, (name, value, expected)
    print("Verified 235 public and 3 explicitly private opcode discriminants")


if __name__ == "__main__":
    main()
