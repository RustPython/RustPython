#!/usr/bin/env python3
"""Ratchet comparable signatures, not runtime argument semantics.

Consumes whats_left's JSON without importing scanned modules or building RustPython.
Baselines contain only signature mismatches and checked-scope fingerprints.
"""

import argparse
import hashlib
import json
from pathlib import Path


def snapshot(result):
    """Check comparable signatures; keep other diagnostics in scan artifacts."""
    defects = {}
    scopes = {}

    def observe(category, scope, name, expected, actual):
        key = f"{category}:{scope}/{name}"
        namespace = scope.partition("/")[0]
        scopes.setdefault(f"{category}:{namespace}", []).append(
            (f"{scope}/{name}", expected)
        )
        if actual != expected:
            defects[key] = [expected, actual]

    raw = result["raw_text_signatures"]
    raw_scopes = {"builtin": raw["builtins"]}
    raw_scopes.update(
        (f"module/{name}", items) for name, items in raw["modules"].items()
    )
    for scope, items in raw_scopes.items():
        for name, pair in items.items():
            ref, actual = pair["cpython"], pair["rustpython"]
            # A failed/absent observation must not be treated as matching text.
            # If a previously checked path becomes unobservable, its scope
            # fingerprint changes and the gate fails instead.
            if (
                ref["status"] == "present"
                and actual["status"] in {"present", "none", "absent"}
                and ref["kind"] == actual["kind"]
                and ref["binding"] == actual["binding"]
                and ref["binding"] not in {"unknown", "bound_non_none"}
            ):
                observe(
                    "raw",
                    scope,
                    name,
                    ref["value"],
                    actual["value"]
                    if actual["status"] == "present"
                    else [actual["status"]],
                )

    mismatches = {
        (group, name): actual
        for group, entries in result["mismatched_items"].items()
        for name, actual, _expected in entries
    }
    inherited = {
        (group, name.removesuffix(" (inherited)"))
        for group, entries in result["missing_items"].items()
        for name in entries
        if name.endswith(" (inherited)")
    }

    def inspected(scope, group, name, full_name, expected):
        if not isinstance(expected, str) or not expected.startswith("("):
            return
        pair = raw_scopes[scope][name]
        if pair["rustpython"]["status"] == "not_observed":
            return
        # The existing scanner reports inherited differences as missing and
        # does not retain their inspected actual value. Do not invent a match.
        if (group, full_name) in inherited:
            return
        actual = mismatches.get((group, full_name), expected)
        # A known API with an invalid/missing inspected signature remains a
        # failure, not an excluded target. Details stay in the scan artifact.
        if not isinstance(actual, str) or not actual.startswith("("):
            actual = ["uncomparable"]
        observe("inspect", scope, name, expected, actual)

    for module, items in result["cpython_modules"].items():
        for name, info in items.items():
            if not info["sig_error"]:
                inspected(
                    f"module/{module}", module, name, f"{module}.{name}", info["sig"]
                )
    for typ, info in result["cpython_builtin_signatures"].items():
        for method, expected in info["methods"].items():
            name = f"{typ}.{method}"
            if result["not_implementeds"].get(typ, {}).get(method) == "(inherited)":
                continue
            inspected("builtin", typ, name, name, expected)

    if (
        result["reference_discovery"]["truncated"]
        or result["reference_inventory_diagnostics"]["truncated"]
    ):
        raise ValueError("truncated reference inventory cannot certify coverage")
    if result["reference_inventory_diagnostics"]["issues"]:
        raise ValueError("ambiguous reference inventory cannot certify signatures")
    if not scopes:
        raise ValueError("empty comparable signature inventory")
    # Store one digest per checked scope, not every unrelated discovery,
    # unknown, import error, missing API or class-selection decision.
    fingerprints = {
        scope: hashlib.sha256(json.dumps(sorted(paths)).encode()).hexdigest()
        for scope, paths in sorted(scopes.items())
    }
    return {"coverage": fingerprints, "defects": defects}


def pack(state):
    # Group identical signature mismatches while retaining individual paths.
    groups = {}
    for path, value in sorted(state["defects"].items()):
        groups.setdefault(json.dumps(value, sort_keys=True), []).append(path)
    return {
        "coverage": state["coverage"],
        "observations": [
            [json.loads(value), paths] for value, paths in sorted(groups.items())
        ],
    }


def unpack(state):
    defects = {}
    for value, paths in state["observations"]:
        for path in paths:
            if path in defects:
                raise ValueError(f"duplicate baseline path: {path}")
            defects[path] = value
    return {"coverage": state["coverage"], "defects": defects}


def compare(current, baseline):
    errors = []
    for scope in sorted(set(current["coverage"]) | set(baseline["coverage"])):
        if current["coverage"].get(scope) != baseline["coverage"].get(scope):
            errors.append(f"checked signature scope changed: {scope}")
    for path, value in sorted(current["defects"].items()):
        if path not in baseline["defects"] or baseline["defects"][path] != value:
            errors.append(
                f"new or changed signature mismatch: {path}: {json.dumps(value)}"
            )
    resolved = sorted(set(baseline["defects"]) - set(current["defects"]))
    errors.extend(f"remove resolved signature exception: {path}" for path in resolved)
    return errors, resolved


def write_baseline(path, baseline):
    # One exact observation group per line keeps generated baseline diffs
    # compact while retaining readable values and individually named paths.
    lines = ["{"]
    for key, value in baseline.items():
        if key == "observations":
            lines.append('  "observations": [')
            lines.extend(
                "    " + json.dumps(row, sort_keys=True) + "," for row in value
            )
            if value:
                lines[-1] = lines[-1].removesuffix(",")
            lines.append("  ],")
        elif key == "coverage":
            lines.append('  "coverage": {')
            lines.extend(
                "    " + json.dumps(scope) + ": " + json.dumps(digest) + ","
                for scope, digest in value.items()
            )
            if value:
                lines[-1] = lines[-1].removesuffix(",")
            lines.append("  },")
        else:
            lines.append(
                "  " + json.dumps(key) + ": " + json.dumps(value, sort_keys=True) + ","
            )
    lines[-1] = lines[-1].removesuffix(",")
    lines.append("}")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument(
        "--context",
        required=True,
        help="OS-architecture-CPython version-build features",
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="explicit maintainer operation; never used by CI",
    )
    args = parser.parse_args()
    current = snapshot(json.loads(args.result.read_text(encoding="utf-8")))
    if args.write_baseline:
        write_baseline(args.baseline, {"context": args.context, **pack(current)})
        return 0
    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    if baseline["context"] != args.context:
        raise ValueError(
            "baseline context differs; collect and review a matching baseline"
        )
    errors, resolved = compare(current, unpack(baseline))
    for error in errors:
        print(error)
    print(
        f"{len(errors)} regressions; {len(resolved)} resolved signature exceptions; {len(current['defects'])} unresolved signature mismatches"
    )
    if resolved:
        print(
            "Remove resolved signature exceptions from the reviewed baseline; unchanged known failures are not compatibility guarantees."
        )
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
