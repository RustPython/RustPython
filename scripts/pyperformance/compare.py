#!/usr/bin/env python3
"""Compare two `run_all.py` catalogs (e.g. RustPython vs. CPython) side by side.

Usage
-----
    # Catalog a real CPython too (no psutil stub needed -- it has a real one):
    python3 scripts/pyperformance/run_all.py --python "$(which python3.13)" \\
        --label cpython3.13 --no-psutil-stub

    # RustPython catalog, if not already generated:
    python3 scripts/pyperformance/run_all.py --python target/release/rustpython \\
        --label rustpython

    # Compare them:
    python3 scripts/pyperformance/compare.py --baseline cpython3.13 --candidate rustpython

Writes `scripts/pyperformance/results/COMPARE-<candidate>-vs-<baseline>.md`.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_RESULTS_DIR = SCRIPT_DIR / "results"

TIME_RE = re.compile(r"^\s*([\d.]+)\s*(ns|us|ms|sec)\b")
UNIT_SECONDS = {"ns": 1e-9, "us": 1e-6, "ms": 1e-3, "sec": 1.0}


def parse_mean_seconds(mean: str | None) -> float | None:
    """Parse a pyperf "<value> <unit> +- <value> <unit>" string into seconds."""
    if not mean:
        return None
    m = TIME_RE.match(mean)
    if not m:
        return None
    value, unit = m.groups()
    return float(value) * UNIT_SECONDS[unit]


def load_catalog(results_dir: Path, label: str) -> dict[str, dict]:
    catalog_json = results_dir / label / "catalog.json"
    if not catalog_json.exists():
        raise SystemExit(
            f"no catalog for label {label!r} at {catalog_json} -- run "
            f"`run_all.py --label {label} ...` first"
        )
    return {r["benchmark"]: r for r in json.loads(catalog_json.read_text())}


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--baseline",
        required=True,
        help="label of the baseline catalog, e.g. cpython3.13",
    )
    parser.add_argument(
        "--candidate",
        required=True,
        help="label of the catalog to compare against it, e.g. rustpython",
    )
    parser.add_argument(
        "--results-dir",
        type=Path,
        default=DEFAULT_RESULTS_DIR,
        help="parent directory containing <label>/catalog.json (default: %(default)s)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="output Markdown file (default: <results-dir>/COMPARE-<candidate>-vs-<baseline>.md)",
    )
    args = parser.parse_args()

    baseline = load_catalog(args.results_dir, args.baseline)
    candidate = load_catalog(args.results_dir, args.candidate)
    names = sorted(set(baseline) | set(candidate))

    rows = []
    both_ok = []
    for name in names:
        b = baseline.get(name)
        c = candidate.get(name)
        b_status = b["status"] if b else "missing"
        c_status = c["status"] if c else "missing"
        b_mean = parse_mean_seconds(b["mean"]) if b else None
        c_mean = parse_mean_seconds(c["mean"]) if c else None
        ratio = None
        if b_mean and c_mean:
            ratio = c_mean / b_mean
            both_ok.append(ratio)
        rows.append(
            {
                "benchmark": name,
                "baseline_status": b_status,
                "baseline_mean": b["mean"] if b else "",
                "candidate_status": c_status,
                "candidate_mean": c["mean"] if c else "",
                "ratio": ratio,
                "candidate_detail": (c or {}).get("detail") or "",
            }
        )

    out = (
        args.out or args.results_dir / f"COMPARE-{args.candidate}-vs-{args.baseline}.md"
    )
    lines = []
    lines.append(f"# {args.candidate} vs {args.baseline} -- pyperformance comparison")
    lines.append("")
    lines.append(
        f"Baseline: **{args.baseline}** ({len([r for r in baseline.values() if r['status'] == 'ok'])}/"
        f"{len(baseline)} passed). Candidate: **{args.candidate}** "
        f"({len([r for r in candidate.values() if r['status'] == 'ok'])}/{len(candidate)} passed)."
    )
    if both_ok:
        both_ok.sort()
        n = len(both_ok)
        median = (
            both_ok[n // 2] if n % 2 else (both_ok[n // 2 - 1] + both_ok[n // 2]) / 2
        )
        lines.append("")
        lines.append(
            f"Median slowdown on the {len(both_ok)} benchmarks both passed: "
            f"**{median:.2f}x** ({args.candidate} time / {args.baseline} time)."
        )
    lines.append("")
    lines.append(
        f"| Benchmark | {args.baseline} | {args.candidate} | {args.candidate}/{args.baseline} | Notes |"
    )
    lines.append("|---|---|---|---|---|")
    for r in rows:
        b_cell = r["baseline_mean"] or f"({r['baseline_status']})"
        c_cell = r["candidate_mean"] or f"({r['candidate_status']})"
        ratio_cell = f"{r['ratio']:.2f}x" if r["ratio"] else ""
        notes = r["candidate_detail"].replace("|", "\\|").replace("\n", " ")
        lines.append(
            f"| {r['benchmark']} | {b_cell} | {c_cell} | {ratio_cell} | {notes} |"
        )
    lines.append("")

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
