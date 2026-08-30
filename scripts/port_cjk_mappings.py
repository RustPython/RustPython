#!/usr/bin/env python3
"""Mechanically translate PyPy cjkcodecs mapping headers to Rust arrays.

The codec state machines stay hand-ported line by line.  This script only
changes the spelling of the generated static data and its page indexes, so the
Rust port keeps PyPy's ``decode_map`` / ``encode_map`` storage shape.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

ARRAY_RE = re.compile(
    r"static const (?P<type>ucs2_t|ucs4_t|DBCHAR|unsigned char) "
    r"(?P<name>[A-Za-z0-9_]+)\[(?P<size>\d*)\] = \{(?P<body>.*?)\n\};",
    re.S,
)
INDEX_RE = re.compile(
    r"static const struct (?P<type>dbcs_index|widedbcs_index|unim_index) "
    r"(?P<name>[A-Za-z0-9_]+)\[256\] = \{(?P<body>.*?)\n\};",
    re.S,
)
GB18030_RANGE_RE = re.compile(
    r"static const struct _gb18030_to_unibmp_ranges \{.*?\} "
    r"gb18030_to_unibmp_ranges\[\] = \{(?P<body>.*?)\};",
    re.S,
)
PAIR_ENCODE_RE = re.compile(
    r"static const struct pair_encodemap (?P<name>[A-Za-z0-9_]+)"
    r"\[(?P<size>[A-Za-z0-9_]+)\] = \{(?P<body>.*?)\n\};",
    re.S,
)

RUST_TYPE = {
    "ucs2_t": "u16",
    "ucs4_t": "u32",
    "DBCHAR": "u16",
    "unsigned char": "u8",
}
SENTINEL = {"U": "UNIINV", "N": "NOCHAR", "M": "MULTIC", "D": "DBCINV"}


def rust_name(name: str) -> str:
    return name.lstrip("_").upper()


def wrap(items: list[str], width: int = 100) -> list[str]:
    lines: list[str] = []
    line = "    "
    for item in items:
        token = item + ","
        if len(line) > 4 and len(line) + len(token) + 1 > width:
            lines.append(line.rstrip())
            line = "    "
        line += token + " "
    if len(line) > 4:
        lines.append(line.rstrip())
    return lines


def translate(source: Path) -> str:
    text = source.read_text()
    output = [
        "// Generated mechanically from PyPy's cjkcodecs mapping header by",
        "// `scripts/port_cjk_mappings.py`.  Do not change the page layout:",
        "// `cjkcodecs.h::_TRYMAP_DEC` / `_TRYMAP_ENC` index these exact arrays.",
        "",
        "pub(super) const UNIINV: u16 = 0xfffe;",
        "#[allow(dead_code)]",
        "pub(super) const NOCHAR: u16 = 0xffff;",
        "#[allow(dead_code)]",
        "pub(super) const MULTIC: u16 = 0xfffe;",
        "#[allow(dead_code)]",
        "pub(super) const DBCINV: u16 = 0xfffd;",
        "",
        "#[derive(Clone, Copy)]",
        "pub(super) struct MapIndex {",
        "    pub(super) offset: usize,",
        "    pub(super) bottom: u8,",
        "    pub(super) top: u8,",
        "    pub(super) present: bool,",
        "}",
        "",
        "const EMPTY_INDEX: MapIndex = MapIndex {",
        "    offset: 0,",
        "    bottom: 0,",
        "    top: 0,",
        "    present: false,",
        "};",
        "",
    ]

    arrays: dict[str, int] = {}
    for match in ARRAY_RE.finditer(text):
        c_name = match.group("name")
        name = rust_name(c_name) + "_DATA"
        values = [
            part.strip() for part in match.group("body").replace("\n", "").split(",")
        ]
        rust_type = RUST_TYPE[match.group("type")]
        values = [
            (
                f"{SENTINEL[value]} as {rust_type}"
                if value in SENTINEL and rust_type != "u16"
                else SENTINEL.get(value, value)
            )
            for value in values
            if value
        ]
        size = int(match.group("size")) if match.group("size") else len(values)
        if len(values) != size:
            raise ValueError(f"{c_name}: declared {size}, parsed {len(values)}")
        arrays[c_name] = size
        output.append(f"pub(super) static {name}: [{rust_type}; {size}] = [")
        output.extend(wrap(values))
        output.extend(["];", ""])

    for match in INDEX_RE.finditer(text):
        c_name = match.group("name")
        name = c_name.upper()
        entries = re.findall(r"\{([^{}]*)\}", match.group("body"))
        if len(entries) != 256:
            raise ValueError(f"{c_name}: expected 256 indexes, parsed {len(entries)}")
        translated: list[str] = []
        for entry in entries:
            compact = re.sub(r"\s+", "", entry)
            pointer, bottom, top = compact.split(",")
            if pointer == "0":
                translated.append("EMPTY_INDEX")
                continue
            array_name, offset = pointer.split("+")
            if array_name not in arrays:
                raise ValueError(f"{c_name}: unknown array {array_name}")
            translated.append(
                "MapIndex { "
                f"offset: {offset}, bottom: {bottom}, top: {top}, present: true "
                "}"
            )
        output.append(f"pub(super) static {name}: [MapIndex; 256] = [")
        output.extend(wrap(translated, 120))
        output.extend(["];", ""])

    if match := GB18030_RANGE_RE.search(text):
        entries = re.findall(
            r"\{\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\}", match.group("body")
        )
        output.extend(
            [
                "#[derive(Clone, Copy)]",
                "pub(super) struct Gb18030Range {",
                "    pub(super) first: u32,",
                "    pub(super) last: u32,",
                "    pub(super) base: u32,",
                "}",
                "",
                f"pub(super) static GB18030_TO_UNIBMP_RANGES: [Gb18030Range; {len(entries)}] = [",
            ]
        )
        output.extend(
            wrap(
                [
                    f"Gb18030Range {{ first: {first}, last: {last}, base: {base} }}"
                    for first, last, base in entries
                ],
                120,
            )
        )
        output.extend(["];", ""])

    if match := PAIR_ENCODE_RE.search(text):
        entries = re.findall(
            r"\{\s*(0x[0-9a-fA-F]+)\s*,\s*(0x[0-9a-fA-F]+)\s*\}", match.group("body")
        )
        size_name = match.group("size")
        size_match = re.search(rf"#define\s+{re.escape(size_name)}\s+(\d+)", text)
        if size_match is None:
            raise ValueError(f"{match.group('name')}: unknown size {size_name}")
        size = int(size_match.group(1))
        if len(entries) != size:
            raise ValueError(
                f"{match.group('name')}: declared {size}, parsed {len(entries)}"
            )
        output.extend(
            [
                "#[derive(Clone, Copy)]",
                "pub(super) struct PairEncodeMap {",
                "    pub(super) uniseq: u32,",
                "    pub(super) code: u16,",
                "}",
                "",
                f"pub(super) static {match.group('name').upper()}: [PairEncodeMap; {size}] = [",
            ]
        )
        output.extend(
            wrap(
                [
                    f"PairEncodeMap {{ uniseq: {uniseq}, code: {code} }}"
                    for uniseq, code in entries
                ],
                120,
            )
        )
        output.extend(["];", ""])

    if not arrays:
        raise ValueError(f"{source}: no mapping arrays found")
    return "\n".join(output)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.write_text(translate(args.source))


if __name__ == "__main__":
    main()
