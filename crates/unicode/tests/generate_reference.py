#!/usr/bin/env python3.14
"""Generate the CPython reference dataset for the differential Unicode sweep.

Run with a CPython interpreter whose ``unicodedata.unidata_version`` matches the
Unicode release this crate targets (16.0.0 for CPython 3.14). The output is a
compact run-length encoding of every predicate's true-set over the full scalar
range, consumed by ``tests/differential.rs``.

Usage:
    python3.14 crates/unicode/tests/generate_reference.py

Writes ``tests/data/cpython3.14_predicates.txt``. Commit the result.
"""

from __future__ import annotations

import pathlib
import sys
import unicodedata

MAX = 0x110000

# str predicates: name -> single-char method.
STR_PREDICATES = {
    "isalpha": str.isalpha,
    "isalnum": str.isalnum,
    "isdecimal": str.isdecimal,
    "isdigit": str.isdigit,
    "isnumeric": str.isnumeric,
    "isspace": str.isspace,
    "isprintable": str.isprintable,
    "isidentifier": str.isidentifier,
}


def encode_ranges(is_true) -> list[tuple[int, int]]:
    """Collapse the true-set of ``is_true`` into inclusive ``[start, end]`` runs."""
    ranges: list[tuple[int, int]] = []
    start: int | None = None
    for cp in range(MAX):
        if is_true(cp):
            if start is None:
                start = cp
        elif start is not None:
            ranges.append((start, cp - 1))
            start = None
    if start is not None:
        ranges.append((start, MAX - 1))
    return ranges


def main() -> int:
    if unicodedata.unidata_version != "16.0.0":
        sys.stderr.write(
            f"warning: unidata_version is {unicodedata.unidata_version}, "
            "expected 16.0.0 (CPython 3.14); regenerating anyway\n"
        )

    out = pathlib.Path(__file__).parent / "data" / "cpython3.14_predicates.txt"
    out.parent.mkdir(parents=True, exist_ok=True)

    lines = [f"# unidata_version {unicodedata.unidata_version}"]
    for name, method in STR_PREDICATES.items():
        ranges = encode_ranges(lambda cp, m=method: m(chr(cp)))
        packed = ",".join(f"{s:X}:{e:X}" for s, e in ranges)
        lines.append(f"{name} {packed}")

    out.write_text("\n".join(lines) + "\n")
    print(f"wrote {out} ({out.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
