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

import _sre
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

# Casing predicates, keyed by the crate function each one exercises. Each is
# sourced from the exact property that function computes:
# * is_lowercase / is_uppercase mirror Py_UNICODE_ISLOWER / ISUPPER, which for a
#   single character are str.islower() / str.isupper().
# * is_titlecase is the Lt general category (NOT str.istitle(), which also
#   reports plain uppercase letters as titlecased).
# * is_cased is the Cased property (Py_UNICODE_ISCASED), via _sre.
CASE_PREDICATES = {
    "is_lowercase": lambda c: c.islower(),
    "is_uppercase": lambda c: c.isupper(),
    "is_titlecase": lambda c: unicodedata.category(c) == "Lt",
    "is_cased": lambda c: _sre.unicode_iscased(ord(c)),
}

# Simple one-to-one lowercase mapping (Py_UNICODE_TOLOWER via _sre). This is the
# mapping the regex IGNORECASE path depends on. Emitted as `cp:mapping,...` for
# code points that map to something other than themselves. CPython exposes no
# Python-level simple-uppercase oracle (_sre has unicode_tolower only), so
# toupper is left to the SRE unit tests.
CASE_MAPPINGS = {
    "tolower": _sre.unicode_tolower,
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

    data = pathlib.Path(__file__).parent / "data"
    data.mkdir(parents=True, exist_ok=True)

    lines = [f"# unidata_version {unicodedata.unidata_version}"]
    for name, method in STR_PREDICATES.items():
        ranges = encode_ranges(lambda cp, m=method: m(chr(cp)))
        packed = ",".join(f"{s:X}:{e:X}" for s, e in ranges)
        lines.append(f"{name} {packed}")
    for name, method in CASE_PREDICATES.items():
        ranges = encode_ranges(lambda cp, m=method: m(chr(cp)))
        packed = ",".join(f"{s:X}:{e:X}" for s, e in ranges)
        lines.append(f"{name} {packed}")

    predicates = data / "cpython3.14_predicates.txt"
    predicates.write_text("\n".join(lines) + "\n")
    print(f"wrote {predicates} ({predicates.stat().st_size} bytes)")

    mapping_lines = [f"# unidata_version {unicodedata.unidata_version}"]
    for name, method in CASE_MAPPINGS.items():
        pairs = [
            f"{cp:X}:{mapped:X}" for cp in range(MAX) if (mapped := method(cp)) != cp
        ]
        mapping_lines.append(f"{name} {','.join(pairs)}")

    mappings = data / "cpython3.14_mappings.txt"
    mappings.write_text("\n".join(mapping_lines) + "\n")
    print(f"wrote {mappings} ({mappings.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
