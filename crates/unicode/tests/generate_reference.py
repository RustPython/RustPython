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


def generate_graphemes(args) -> int:
    """Write a new Unicode 17 oracle, without touching the older fixtures."""
    import hashlib
    import json
    import subprocess

    commit = "d220409a13b77de7a6c30abe16ea90db01528edc"
    gcb_names = (
        "Other",
        "Prepend",
        "CR",
        "LF",
        "Control",
        "Extend",
        "Regional_Indicator",
        "SpacingMark",
        "L",
        "V",
        "T",
        "LV",
        "LVT",
        "ZWJ",
    )
    incb_names = ("None", "Linker", "Consonant", "Extend")

    def digest(path):
        h = hashlib.sha256()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(65536), b""):
                h.update(chunk)
        return h.hexdigest()

    if sys.implementation.name != "cpython" or unicodedata.unidata_version != "17.0.0":
        raise RuntimeError("requires the reviewed CPython Unicode 17 oracle")
    executable = pathlib.Path(sys.executable).resolve(strict=True)
    extension = pathlib.Path(unicodedata.__file__).resolve(strict=True)
    if (
        digest(executable) != args.python_sha256
        or digest(extension) != args.unicodedata_sha256
    ):
        raise RuntimeError("oracle executable or extension hash mismatch")
    source = args.cpython_source.resolve(strict=True)
    head = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"],
        timeout=5,
        text=True,
    ).strip()
    if head != commit:
        raise RuntimeError("CPython source is not the pinned final commit")
    header = source / "Modules/unicodedata_db.h"
    blob = subprocess.check_output(
        ["git", "-C", str(source), "show", f"{commit}:Modules/unicodedata_db.h"],
        timeout=5,
    )
    if digest(header) != hashlib.sha256(blob).hexdigest():
        raise RuntimeError("CPython Unicode database header differs from pinned source")

    outputs = [
        args.graphemes_output,
        args.graphemes_raw_output,
        args.graphemes_manifest,
    ]
    outputs = [p.resolve() for p in outputs]
    if len(set(outputs)) != 3:
        raise RuntimeError("oracle output paths must be distinct")
    old_data = pathlib.Path(__file__).resolve().parent / "data"
    for path in outputs:
        if path.exists() or path.is_relative_to(old_data):
            raise RuntimeError(
                "outputs must be new files outside existing fixture directory"
            )
        if not path.parent.is_dir():
            raise RuntimeError("output parents must already exist")

    range_path, raw_path, manifest_path = outputs
    ranges = []
    start = 0
    previous = None
    raw_hash = hashlib.sha256()
    # Exclusive creation prevents replacing any existing reference artifact.
    with raw_path.open("xb") as raw:
        buffer = bytearray()
        for cp in range(MAX):
            c = chr(cp)
            triple = (
                gcb_names.index(unicodedata.grapheme_cluster_break(c)),
                incb_names.index(unicodedata.indic_conjunct_break(c)),
                int(unicodedata.extended_pictographic(c)),
            )
            if triple != previous:
                if previous is not None:
                    ranges.append((start, cp - 1, previous))
                    if len(ranges) > 100000:
                        raise RuntimeError("range oracle exceeds reviewed count bound")
                start, previous = cp, triple
            buffer.extend(triple)
            if len(buffer) >= 65535:
                raw.write(buffer)
                raw_hash.update(buffer)
                buffer.clear()
        raw.write(buffer)
        raw_hash.update(buffer)
    ranges.append((start, MAX - 1, previous))
    if raw_path.stat().st_size != MAX * 3:
        raise RuntimeError("raw oracle length mismatch")
    lines = [
        f"# CPython {commit}; Unicode 17.0.0",
        "# Inclusive contiguous ranges: start:end GCB InCB Extended_Pictographic",
        f"# Reconstructed raw SHA256 {raw_hash.hexdigest()}",
    ]
    for start, end, (gcb, incb, pictographic) in ranges:
        lines.append(
            f"{start:06X}:{end:06X} {gcb_names[gcb]} {incb_names[incb]} {pictographic}"
        )
    text = ("\n".join(lines) + "\n").encode("ascii")
    if len(text) > 1024 * 1024 or len(ranges) > 100000:
        raise RuntimeError("range oracle exceeds reviewed size bound")
    reconstructed = hashlib.sha256()
    next_cp = 0
    for line in text.decode("ascii").splitlines():
        if line.startswith("#"):
            continue
        bounds, gcb, incb, pictographic = line.split()
        first, last = (int(value, 16) for value in bounds.split(":"))
        if first != next_cp or not first <= last < MAX:
            raise RuntimeError("range encoding has a gap, overlap, or invalid bound")
        triple = bytes(
            (gcb_names.index(gcb), incb_names.index(incb), int(pictographic))
        )
        reconstructed.update(triple * (last - first + 1))
        next_cp = last + 1
    if next_cp != MAX or reconstructed.hexdigest() != raw_hash.hexdigest():
        raise RuntimeError("range reconstruction differs from the raw property oracle")
    with range_path.open("xb") as output:
        output.write(text)
    manifest = {
        "cpython_commit": commit,
        "unicode_version": unicodedata.unidata_version,
        "runtime_version": sys.version,
        "runtime_git": getattr(sys, "_git", None),
        "python": str(executable),
        "python_sha256": digest(executable),
        "unicodedata": str(extension),
        "unicodedata_sha256": digest(extension),
        "database_header_sha256": digest(header),
        "generator_sha256": digest(pathlib.Path(__file__)),
        "codepoints": MAX,
        "properties": 3,
        "ranges": len(ranges),
        "gcb_encoding": gcb_names,
        "incb_encoding": incb_names,
        "raw_bytes": MAX * 3,
        "raw_sha256": raw_hash.hexdigest(),
        "ranges_bytes": len(text),
        "ranges_sha256": hashlib.sha256(text).hexdigest(),
    }
    manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode("utf-8")
    # Total artifact bound, independent of the launcher's per-file rlimit.
    if (
        len(manifest_bytes) > 16384
        or MAX * 3 + len(text) + len(manifest_bytes) > 4_407_296
    ):
        raise RuntimeError("total oracle artifact budget exceeded")
    with manifest_path.open("xb") as output:
        output.write(manifest_bytes)
    print(json.dumps(manifest, sort_keys=True))
    return 0


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
    if len(sys.argv) == 1:
        raise SystemExit(main())
    import argparse

    parser = argparse.ArgumentParser(
        description="Generate only a new grapheme property oracle"
    )
    parser.add_argument("--graphemes-only", action="store_true", required=True)
    parser.add_argument("--graphemes-output", type=pathlib.Path, required=True)
    parser.add_argument("--graphemes-raw-output", type=pathlib.Path, required=True)
    parser.add_argument("--graphemes-manifest", type=pathlib.Path, required=True)
    parser.add_argument("--cpython-source", type=pathlib.Path, required=True)
    parser.add_argument("--python-sha256", required=True)
    parser.add_argument("--unicodedata-sha256", required=True)
    raise SystemExit(generate_graphemes(parser.parse_args()))
