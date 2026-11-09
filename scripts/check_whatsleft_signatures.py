#!/usr/bin/env python3
"""Ratchet the *observed* signature surface, not runtime argument semantics.

Consumes whats_left's JSON without importing scanned modules or building RustPython.
Baselines are platform-specific observations, never wildcard suppressions.
"""

import argparse
import hashlib
import json
import re
from pathlib import Path

GOOD_RAW = {
    "exact_raw_match",
    "matching_nontext_state",
    "not_applicable",
    "additional_signature",
}
RAW_FIELDS = ("status", "kind", "binding", "value", "error_type", "representative")


def additional_signature(pair, comparison):
    # Accept older scanner artifacts too. No reference text means there is
    # no text to disagree with. Retain the
    # observation/coverage, but do not require an exception for useful extra
    # RustPython metadata. Unknown or different contexts remain unverified.
    return (
        comparison == "different_text_state"
        and pair["cpython"]["status"] in {"none", "absent"}
        and pair["rustpython"]["status"] == "present"
    )


def diagnostic(value):
    # Only exception diagnostics contain process-specific repr addresses.
    # Never normalize raw signature literals, parameter names or defaults.
    if isinstance(value, str) and re.match(r"[A-Za-z_][\w.]*\(", value):
        return re.sub(r" at 0x[0-9a-fA-F]+(?=>)", " at 0xADDR", value)
    return value


def snapshot(result):
    # Require the schema: a missing collection must not look like zero failures.
    defects = {}
    coverage = set()

    def observe(category, key, value, good=False):
        name = f"{category}:{key}"
        coverage.add(name)
        if not good:
            defects[name] = value

    raw = result["raw_text_signatures"]
    scopes = [("builtin", raw["builtins"])]
    scopes += [(f"module/{name}", items) for name, items in raw["modules"].items()]
    for scope, items in scopes:
        for name, pair in items.items():
            comparison = pair["comparison"]
            # Type-document equality is not constructor compatibility. Keep its
            # observation separate but still guard its text against regressions.
            kind = comparison.removeprefix("type_document_")
            observe(
                "raw",
                f"{scope}/{name}",
                [comparison]
                + [
                    [pair[side][field] for field in RAW_FIELDS]
                    for side in ("cpython", "rustpython")
                ],
                kind in GOOD_RAW or additional_signature(pair, kind),
            )

    # The complete selected reference inventory protects even paths that did
    # not produce a comparable signature (and aliases remain separate paths).
    for module, items in result["cpython_modules"].items():
        for name, info in items.items():
            observe(
                "reference",
                f"{module}/{name}",
                diagnostic(info["sig"]),
                not info["sig_error"],
            )
    for name, methods in result["reference_builtin_inventory"].items():
        coverage.add(f"builtin:{name}")
        coverage.update(f"builtin:{name}.{method}" for method in methods)
    for group, entries in result["mismatched_items"].items():
        for name, actual, expected in entries:
            observe(
                "inspect", f"{group}/{name}", [diagnostic(expected), diagnostic(actual)]
            )
    for group, entries in result["uncomparable_signatures"].items():
        for name, error in entries:
            observe("uncomparable", f"{group}/{name}", diagnostic(error))
    # Inherited differences are classified as missing, not mismatched_items.
    for group, entries in result["missing_items"].items():
        for name in entries:
            observe("missing", f"{group}/{name}", True)
    for name, entries in result["not_implementeds"].items():
        for method, reason in entries.items():
            observe("missing_builtin", f"{name}.{method}", reason)
    for field, good in (
        ("actual_scan_status", "scan_completed"),
        ("actual_second_import_status", "returned_nonexception"),
    ):
        for name, status in result[field].items():
            observe(field, name, status, status == good)
    for name, entries in result["actual_parent_scans"].items():
        for parent, status in entries.items():
            observe("parent", f"{name}/{parent}", status, status == "real_type")

    for field in ("actual_parent_decisions", "reference_class_selection"):
        for module, items in result[field].items():
            for item, status in items.items():
                coverage.add(f"{field}_decision:{module}/{item}={status}")
                observe(
                    field,
                    f"{module}/{item}",
                    status,
                    status in {"isinstance_true", "selected", "not_selected"},
                )
    for module, parents in result["reference_expanded_classes"].items():
        coverage.update(f"expanded:{module}/{parent}" for parent in parents)
    for module, error in result["cpython_failed_imports"].items():
        observe("reference_import", module, diagnostic(error))

    discovery = result["reference_discovery"]
    diagnostics = result["reference_inventory_diagnostics"]
    if discovery["truncated"] or diagnostics["truncated"]:
        raise ValueError("truncated reference inventory cannot certify coverage")
    for name in discovery["names"]:
        coverage.add(f"discovered:{name}")
    for key, value in (
        ("scanner_errors", discovery["scanner_errors"]),
        ("inventory_issues", diagnostics["issues"]),
    ):
        if value:
            defects[key] = value
    if not raw["builtins"] or not raw["modules"]:
        raise ValueError("empty raw inventory cannot certify coverage")
    # Hash each exact selected scope instead of checking in tens of thousands
    # of duplicate successful observations. Additions also require an explicit
    # reviewed refresh: a changed inventory must never silently certify parity.
    scopes = {}
    failure_prefixes = ("inspect:", "uncomparable:", "missing:", "missing_builtin:")
    for path in sorted(coverage):
        if path.startswith(failure_prefixes):
            continue
        category, _, item = path.partition(":")
        parent = item.rpartition("/")[0]
        scopes.setdefault(f"{category}:{parent}", []).append(item)
    fingerprints = {
        scope: hashlib.sha256(json.dumps(paths).encode()).hexdigest()
        for scope, paths in sorted(scopes.items())
    }
    return {"coverage": fingerprints, "defects": defects}


def pack(state):
    # Group identical observations to avoid repeating thousands of unknown or
    # missing states. Every suppression still names each individual API path.
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
            errors.append(f"observation scope changed: {scope}")
    for path, value in sorted(current["defects"].items()):
        if path not in baseline["defects"] or baseline["defects"][path] != value:
            errors.append(f"new or changed observation: {path}: {json.dumps(value)}")
    resolved = sorted(set(baseline["defects"]) - set(current["defects"]))
    errors.extend(f"remove resolved baseline observation: {path}" for path in resolved)
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
        f"{len(errors)} regressions; {len(resolved)} resolved baseline observations; {len(current['defects'])} unresolved observations"
    )
    if resolved:
        print(
            "Remove resolved entries when updating the reviewed baseline; unchanged known failures are not compatibility guarantees."
        )
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
