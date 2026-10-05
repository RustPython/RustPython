"""Generate Windows C API exports from implemented stable-ABI definitions.

Maintenance dependencies: Python 3.11+, Git, the pinned CPython checkout, and
RustPython source. Compiled verification additionally needs GNU nm and a matching
ELF cdylib. Source-only generation explicitly leaves compiled verification open.
"""

import argparse
import hashlib
import re
import subprocess
import sys
from pathlib import Path

import tomllib

CPYTHON_REVISION = "8a8eb0b90d40f0e03f8252e9f73cad156e1caca8"
MANIFEST_SHA256 = "eab726f2482c71e67a23ea04aaa9c5a11d565bee4472320f813262f273c1e966"
EXPORT_ATTR = "#[unsafe(no_mangle)]"
ITEM_PREFIX = r"#\[unsafe\(no_mangle\)\](?:\s*#\[[^\n]*\])*\s*pub\s+"


def fail(message):
    raise SystemExit(message)


def source_symbols(repo):
    symbols = {}
    macro_templates = {"object.rs": 2, "pyerrors.rs": 1, "monitoring.rs": 1}
    for path in sorted((repo / "crates/capi/src").rglob("*.rs")):
        source, _, tests = path.read_text().partition("#[cfg(test)]")
        if EXPORT_ATTR in tests:
            fail(f"Review exports inside/after the test module: {path}")
        if "export_name" in source:
            fail(f"Review new export_name syntax: {path}")
        functions = re.findall(
            ITEM_PREFIX + r'(?:unsafe\s+)?extern\s+"C"\s+fn\s+([A-Za-z_]\w*)',
            source,
        )
        data = re.findall(
            ITEM_PREFIX + r"static\s+(?:mut\s+)?((?!mut\b)[A-Za-z_]\w*)\s*:",
            source,
        )
        accounted = len(functions) + len(data) + macro_templates.get(path.name, 0)
        if source.count(EXPORT_ATTR) != accounted:
            fail(f"Unrecognized no_mangle item or macro template: {path}")
        functions += re.findall(r"define_py_check!\(\s*(?:exact\s+)?fn\s+(\w+)", source)
        functions += re.findall(r"^fire_event!\((\w+)", source, re.MULTILINE)
        data += re.findall(r"^\s*(PyExc_\w+)\s*=>", source, re.MULTILINE)
        # Current production cfg only selects private FfiResult implementations.
        # New conditional modules/exports require an explicit target mapping.
        if re.search(r"#\[cfg(?:_attr)?\b", source) and (
            path.name != "util.rs" or functions or data or re.search(r"\bmod\s", source)
        ):
            fail(f"Production cfg needs an explicit target-aware review: {path}")
        for kind, names in (("function", functions), ("data", data)):
            for name in names:
                if name in symbols:
                    fail(f"Duplicate source export: {name}")
                symbols[name] = kind
    if not symbols:
        fail("No source C API exports found")
    return symbols


def compiled_symbols(library, nm):
    output = subprocess.check_output(
        [nm, "-D", "--defined-only", str(library)], text=True
    )
    symbols = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 3 or not fields[2].startswith(("Py", "_Py")):
            continue
        _, kind, name = fields
        if kind in "TW":
            symbols[name] = "function"
        elif kind in "BDGRSV":
            symbols[name] = "data"
        else:
            fail(f"Unrecognized nm symbol kind: {line}")
    if not symbols:
        fail("No C API definitions found in the supplied ELF library")
    return symbols


def generate(args):
    manifest_bytes = (args.cpython / "Misc/stable_abi.toml").read_bytes()
    revision = subprocess.check_output(
        ["git", "-C", str(args.cpython), "rev-parse", "HEAD"], text=True
    ).strip()
    if revision != CPYTHON_REVISION:
        fail(f"CPython reference must be {CPYTHON_REVISION}, got {revision}")
    if hashlib.sha256(manifest_bytes).hexdigest() != MANIFEST_SHA256:
        fail("The pinned stable ABI manifest has changed")
    manifest = tomllib.loads(manifest_bytes.decode())
    source = source_symbols(args.repo)
    if args.source_only:
        if args.allow_unbuilt:
            fail("--allow-unbuilt requires an actual --library comparison")
        print("SOURCE-ONLY: ELF and PE verification are pending", file=sys.stderr)
    else:
        compiled = compiled_symbols(args.library, args.nm)
        missing = source.keys() - compiled.keys()
        unexpected = compiled.keys() - source.keys()
        allowed = set(args.allow_unbuilt)
        if missing != allowed or unexpected:
            fail(
                f"Source/library mismatch: unbuilt={sorted(missing)}, stale={sorted(unexpected)}"
            )
        for name in source.keys() & compiled.keys():
            if source[name] != compiled[name]:
                fail(f"Source/library kind mismatch: {name}")
        if allowed:
            print(
                f"UNVERIFIED compiled definitions: {', '.join(sorted(allowed))}",
                file=sys.stderr,
            )
        print(f"{len(compiled)} compiled exports compared")
    exports = {}
    for name, kind in source.items():
        entry = manifest.get(kind, {}).get(name)
        if entry is None:
            continue
        if "ifdef" in entry:
            fail(f"Review target condition for {name}: {entry['ifdef']}")
        exports[name] = kind
    contents = (
        "; Generated by scripts/generate_capi_exports.py; do not edit by hand.\n"
        f"; CPython v3.15.0rc3: {CPYTHON_REVISION}\n"
        f"; Misc/stable_abi.toml SHA-256: {MANIFEST_SHA256}\n"
        "; Only implemented stable-ABI functions/data, including ABI-only names.\n"
        "EXPORTS\n"
    ) + "".join(
        f"    {name}{' DATA' if kind == 'data' else ''}\n"
        for name, kind in sorted(exports.items())
    )
    if args.check:
        if args.output.read_text() != contents:
            fail(f"Export list is stale: {args.output}")
    else:
        args.output.write_text(contents)
    print(
        f"{len(source)} source exports; {len(exports)} stable Windows exports "
        f"({sum(kind == 'data' for kind in exports.values())} data)"
    )


def verify_pe(args):
    expected = {
        line.split()[0]
        for line in args.definition.read_text().splitlines()
        if line.strip() and not line.startswith(";") and line.strip() != "EXPORTS"
    }
    actual = set(
        re.findall(
            r"^\s*\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]+\s+((?:_?Py)\w*)\b",
            args.dumpbin.read_text(encoding="utf-8-sig"),
            re.MULTILINE,
        )
    )
    if actual != expected:
        fail(
            f"PE export mismatch: missing={sorted(expected - actual)}, unexpected={sorted(actual - expected)}"
        )
    print(f"Verified {len(actual)} C API names in the Windows PE export table")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    generation = modes.add_parser("generate")
    generation.add_argument(
        "--repo", type=Path, default=Path(__file__).resolve().parents[1]
    )
    generation.add_argument("--cpython", type=Path, required=True)
    evidence = generation.add_mutually_exclusive_group(required=True)
    evidence.add_argument("--library", type=Path)
    evidence.add_argument("--source-only", action="store_true")
    generation.add_argument("--nm", default="nm")
    generation.add_argument("--output", type=Path, required=True)
    generation.add_argument("--check", action="store_true")
    generation.add_argument(
        "--allow-unbuilt", action="append", default=[], choices=["PyCFunction_Call"]
    )
    pe = modes.add_parser("verify-pe")
    pe.add_argument("--definition", type=Path, required=True)
    pe.add_argument("--dumpbin", type=Path, required=True)
    args = parser.parse_args()
    if args.mode == "generate":
        generate(args)
    else:
        verify_pe(args)


if __name__ == "__main__":
    main()
