#!/usr/bin/env python3
"""Render a base/head/CPython pyperformance comparison as a PR comment.

`compare.py` puts one catalog next to one other catalog, which is the right
shape for "how far is RustPython from CPython". A pull request needs a third
column -- the base commit -- and it needs all three measured *in the same CI
job*, back to back on one runner: a ratio between two numbers that were
measured on different machines can move by more than the change under review
did, so a cross-run comparison cannot tell a real regression from a runner
that happened to be slower that day.

Reads `<results-dir>/<label>/catalog.json` for the three labels and writes a
Markdown comment body (and optionally the same data as JSON).

Usage:

    python3 scripts/pyperformance/pr_diff.py \\
        --cpython cpython --base base --head head \\
        --results-dir "$RUNNER_TEMP/pyperf-results" \\
        --md diff.md --json diff.json
"""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path

from compare import load_catalog, parse_mean_seconds

MARKER = "<!-- pyperformance-local-diff -->"

# Below this, a head/base difference is indistinguishable from run-to-run
# noise on a shared CI runner, so it is rendered as "~" rather than a number
# that invites over-reading.
NOISE = 0.03


def fmt_ratio(value: float | None) -> str:
    return "-" if value is None else f"{value:.2f}x"


def fmt_change(base: float | None, head: float | None) -> str:
    """Head vs. base as a percentage, blanked out inside the noise floor."""
    if not base or not head:
        return "-"
    change = head / base - 1.0
    if abs(change) < NOISE:
        return "~"
    return f"{change * 100:+.1f}%"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--cpython", default="cpython", help="CPython catalog label")
    parser.add_argument("--base", default="base", help="base-commit catalog label")
    parser.add_argument("--head", default="head", help="head-commit catalog label")
    parser.add_argument(
        "--results-dir",
        type=Path,
        required=True,
        help="parent directory containing <label>/catalog.json",
    )
    parser.add_argument("--md", type=Path, required=True, help="Markdown output file")
    parser.add_argument("--json", type=Path, help="optional JSON output file")
    parser.add_argument(
        "--cpython-version",
        default="CPython",
        help="version string to label the CPython column with",
    )
    return parser.parse_args()


def load_base_catalog(results_dir: Path, label: str) -> tuple[dict, bool]:
    """Load the base-commit catalog, if any.

    The base commit is the column that makes this a review aid rather than a
    status report, but it is also the one that can go missing: it may fail to
    build, or predate something the benchmarks now need. The caller falls
    back to head against CPython alone when nothing here produced a usable
    time, rather than failing the whole comparison.
    """
    base = {}
    if (results_dir / label / "catalog.json").exists():
        base = load_catalog(results_dir, label)
    have_base = any(parse_mean_seconds(r.get("mean")) for r in base.values())
    return base, have_base


def build_rows(cpython: dict, base: dict, head: dict) -> list[dict]:
    rows = []
    for name in sorted(set(cpython) | set(base) | set(head)):
        c = parse_mean_seconds((cpython.get(name) or {}).get("mean"))
        b = parse_mean_seconds((base.get(name) or {}).get("mean"))
        h = parse_mean_seconds((head.get(name) or {}).get("mean"))
        rows.append(
            {
                "benchmark": name,
                "cpython_mean": (cpython.get(name) or {}).get("mean") or "",
                "base_mean": (base.get(name) or {}).get("mean") or "",
                "head_mean": (head.get(name) or {}).get("mean") or "",
                "base_status": (base.get(name) or {}).get("status", "missing"),
                "head_status": (head.get(name) or {}).get("status", "missing"),
                "base_vs_cpython": (b / c) if (b and c) else None,
                "head_vs_cpython": (h / c) if (h and c) else None,
                "head_vs_base": (h / b) if (h and b) else None,
            }
        )
    return rows


def render_intro(have_base: bool, cpython_version: str) -> list[str]:
    if have_base:
        return [
            "All three interpreters were measured back to back in this one job, "
            "so they share the exact same CPU and kernel. Ratios are "
            f"`time / {cpython_version} time` -- lower is better. A "
            f"head/base difference smaller than {NOISE:.0%} is shown as `~`; CI "
            "runners are not quiet enough to read more into it than that."
        ]
    return [
        "No usable result for the base commit -- it did not build, or none "
        "of its benchmarks produced a time -- so this is head against "
        f"{cpython_version} alone, both measured back to back in this "
        "one job. Ratios are `time / {} time`; lower is better.".format(cpython_version)
    ]


def render_summary_panel(
    rows: list[dict], have_base: bool, cpython_version: str
) -> list[str]:
    """The "| | base | head |" mini-table: pass counts, the median slowdown
    vs. CPython, and the head/base geometric mean.

    Each row is independently gated on having the data it needs: the two
    medians are restricted to benchmarks with a valid ratio on *both* sides,
    so a benchmark that regressed to failure on only one side can't drop out
    of only that side's population and skew the comparison; "Benchmarks
    passed" and the geometric mean don't depend on CPython at all, so they
    must not disappear just because no benchmark had a usable CPython
    comparison.
    """
    if not have_base:
        return []

    common_vs_cpython = [
        r for r in rows if r["base_vs_cpython"] and r["head_vs_cpython"]
    ]
    base_ratios = [r["base_vs_cpython"] for r in common_vs_cpython]
    head_ratios = [r["head_vs_cpython"] for r in common_vs_cpython]
    base_passed = sum(1 for r in rows if r["base_status"] == "ok")
    head_passed = sum(1 for r in rows if r["head_status"] == "ok")
    both = [r for r in rows if r["head_vs_base"]]

    out = ["| | base | head |", "| --- | ---: | ---: |"]
    if base_ratios and head_ratios:
        out.append(
            f"| Median slowdown vs. {cpython_version} "
            f"({len(common_vs_cpython)} common) "
            f"| {statistics.median(base_ratios):.2f}x "
            f"| {statistics.median(head_ratios):.2f}x |"
        )
    out.append(f"| Benchmarks passed | {base_passed} | {head_passed} |")
    if both:
        geo = statistics.geometric_mean([r["head_vs_base"] for r in both])
        out.append(f"| Geometric mean, head/base ({len(both)} common) | | {geo:.3f} |")
    out.append("")
    return out


def render_table(rows: list[dict], have_base: bool, cpython_version: str) -> list[str]:
    if have_base:
        out = [
            f"| Benchmark | {cpython_version} | base | head "
            f"| base/{cpython_version} | head/{cpython_version} "
            "| head vs. base |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    else:
        out = [
            f"| Benchmark | {cpython_version} | head | head/{cpython_version} |",
            "| --- | ---: | ---: | ---: |",
        ]

    for r in sorted(rows, key=lambda r: r["head_vs_base"] or r["head_vs_cpython"] or 0):
        # A row with no mean on either side but a fail/timeout status is still
        # worth showing -- e.g. a C-extension gap that fails identically on
        # base and head. Only a benchmark never attempted on either commit
        # (no entry in that catalog at all) has nothing to report.
        if r["base_status"] == "missing" and r["head_status"] == "missing":
            continue
        head_cell = r["head_mean"] or "({})".format(r["head_status"])
        if not have_base:
            out.append(
                f"| {r['benchmark']} | {r['cpython_mean'] or '-'} | {head_cell} "
                f"| {fmt_ratio(r['head_vs_cpython'])} |"
            )
            continue
        base_cell = r["base_mean"] or "({})".format(r["base_status"])
        out.append(
            f"| {r['benchmark']} | {r['cpython_mean'] or '-'} "
            f"| {base_cell} | {head_cell} "
            f"| {fmt_ratio(r['base_vs_cpython'])} | {fmt_ratio(r['head_vs_cpython'])} "
            f"| {fmt_change(1.0, r['head_vs_base'])} |"
        )
    return out


def find_transitions(rows: list[dict]) -> tuple[list[str], list[str]]:
    """Benchmarks whose pass/fail status flipped between base and head.

    A transition claim needs an actual *fail/timeout* status on the other
    side, not just a missing entry: a benchmark never attempted there (e.g.
    a partial --benchmarks run) isn't a regression or a fix, just no data
    point. This intentionally doesn't gate on `have_base` (whether *any*
    base benchmark produced a timing) -- a benchmark can still validly flip
    from failing to passing even when every other base benchmark failed too.
    """
    only_head = [
        r["benchmark"]
        for r in rows
        if r["head_status"] == "ok" and r["base_status"] in ("fail", "timeout")
    ]
    only_base = [
        r["benchmark"]
        for r in rows
        if r["base_status"] == "ok" and r["head_status"] in ("fail", "timeout")
    ]
    return only_head, only_base


def main() -> None:
    args = parse_args()

    cpython = load_catalog(args.results_dir, args.cpython)
    head = load_catalog(args.results_dir, args.head)
    base, have_base = load_base_catalog(args.results_dir, args.base)
    rows = build_rows(cpython, base, head)

    title = (
        "base vs. head vs. CPython (same runner)"
        if have_base
        else "head vs. CPython (same runner)"
    )
    out = [MARKER, f"### pyperformance: {title}", ""]
    out += render_intro(have_base, args.cpython_version)
    out.append("")
    out += render_summary_panel(rows, have_base, args.cpython_version)
    out += render_table(rows, have_base, args.cpython_version)

    only_head, only_base = find_transitions(rows)
    if only_head:
        out += ["", f"Newly passing on head: {', '.join(only_head)}."]
    if only_base:
        out += ["", f"No longer passing on head: {', '.join(only_base)}."]

    args.md.write_text("\n".join(out) + "\n")
    if args.json:
        args.json.write_text(json.dumps(rows, indent=2) + "\n")


if __name__ == "__main__":
    main()
