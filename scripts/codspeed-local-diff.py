#!/usr/bin/env python3
"""Measure and diff instruction counts for base and head on the same runner.

CodSpeed's own comparison is cross-run: the base was measured on whatever
machine drew that job, the head on whatever machine draws this one, and a
hosted runner pool hands out a different machine each time. glibc resolves
`memcpy`/`malloc`/the string routines through IFUNC selectors keyed on CPU
flags, so two machines run different code for the same binary and a
Valgrind/Callgrind instruction count moves with that even though it does not
move with clock speed (see
https://codspeed.io/blog/why-glibc-faster-github-actions). `main` and a PR's
head measured on two different runners can disagree by noise the SaaS then
reports as a regression of the branch that happened to draw the other
machine.

This script removes the cross-run part of that: it is run twice within one
job, once against the base commit's bench binaries and once against head's,
so both measurements execute under the identical kernel, glibc and CPU. It
shells out to CodSpeedHQ's own `codspeed run` CLI with `--skip-upload` for
the actual measurement -- the bench binaries `cargo codspeed build` produces
only report real instruction counts when driven by that CLI's runner
protocol (a FIFO handshake the `instrument-hooks` library baked into the
binary expects; a bare `valgrind --tool=callgrind` around the binary
produces a well-formed but permanently-empty `summary: 0`, confirmed while
building this). Nothing is uploaded anywhere. Counts are per `[[bench]]`
target, not per individual benchmark, and are not comparable to CodSpeed's
own dashboard numbers, but the base-vs-head ratio computed here does not
carry the cross-machine noise a SaaS comparison would.

    codspeed-local-diff.py measure --out results/head
    codspeed-local-diff.py measure --out results/base
    codspeed-local-diff.py diff --base results/base --head results/head \
        --md diff.md --json diff.json
"""

import argparse
import glob
import json
import os
import re
import subprocess
import sys
from pathlib import Path

PACKAGES = ("rustpython", "rustpython-sre_engine")

# A target whose count moved by less than this is not called out as a
# regression -- Valgrind/Callgrind is deterministic given identical code, but
# a handful of instructions can still differ run to run from things like
# environment-dependent allocation addresses feeding into hash iteration
# order. Anything below this is noise, not signal.
NOISE_FLOOR_PCT = 0.5


def _cargo_metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version=1"],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def _bench_binaries(metadata):
    """Yield (package, bench_target_name, path) for every built simulation binary.

    `cargo-codspeed` names this directory after its internal `BuildMode`, not
    the `--measurement-mode` flag: `simulation` (and `memory`) both build in
    `BuildMode::Analysis`, which it puts under `target/codspeed/analysis/`,
    not `target/codspeed/simulation/`.
    """
    target_dir = Path(metadata["target_directory"]) / "codspeed" / "analysis"
    packages_by_name = {p["name"]: p for p in metadata["packages"]}
    for package_name in PACKAGES:
        package = packages_by_name.get(package_name)
        if package is None:
            continue
        manifest_dir = Path(package["manifest_path"]).parent
        package_dir = target_dir / package_name
        if not package_dir.is_dir():
            continue
        for entry in sorted(package_dir.iterdir()):
            if entry.is_file():
                yield package_name, entry.name, entry, manifest_dir


def _parse_callgrind_ir(path):
    """Get the file's total Ir (instructions retired) via `callgrind_annotate`.

    A callgrind output file's own per-dump `summary:` lines are not reliable
    here: with `instrument-hooks` dumping once per benchmark (via
    CALLGRIND_DUMP_STATS) a file holds many dumps, and every one of them
    reads `summary: 0` even on a multi-hundred-megabyte file that plainly
    holds real per-line cost records (confirmed on moreal/RustPython#25) --
    apparently the per-dump summary annotation just isn't trustworthy under
    this dump pattern. `callgrind_annotate` (shipped with Valgrind) computes
    the total the same way any other consumer of this format would: by
    summing the actual per-line cost records, and prints it as a
    `PROGRAM TOTALS` row whose columns are in the same order as the file's
    `events:` header.
    """
    out = subprocess.run(
        ["callgrind_annotate", "--threshold=0", str(path)],
        capture_output=True,
        text=True,
        check=True,
    )
    for line in out.stdout.splitlines():
        if line.rstrip().endswith("PROGRAM TOTALS"):
            first_column = line.split()[0]
            return int(first_column.replace(",", ""))
    raise ValueError(
        f"callgrind_annotate produced no PROGRAM TOTALS line for {path}; "
        f"stdout was:\n{out.stdout}\nstderr was:\n{out.stderr}"
    )


def measure(out_dir, bench_filter=None):
    metadata = _cargo_metadata()
    binaries = list(_bench_binaries(metadata))
    if bench_filter:
        binaries = [b for b in binaries if bench_filter in b[1]]
    if not binaries:
        raise SystemExit(
            "No built simulation benchmarks found; run "
            "`cargo codspeed build --measurement-mode simulation` first."
        )

    # Each bench binary runs with its package's manifest directory as cwd
    # (via --working-directory, to match how `cargo codspeed run` invokes
    # it), so a relative --out here would land under a different directory
    # for every package.
    out_dir = Path(out_dir).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    results = {}

    workspace_root = Path(metadata["workspace_root"])
    for package_name, bench_name, bench_path, manifest_dir in binaries:
        target_key = f"{package_name}/{bench_name}"
        safe_key = re.sub(r"[^A-Za-z0-9_.-]", "_", target_key)
        # A fresh subdirectory per target: `codspeed run` names its own
        # per-process callgrind files, and this keeps two targets' files
        # from colliding without us having to guess that naming.
        target_out_dir = out_dir / safe_key
        target_out_dir.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ)
        env["CODSPEED_CARGO_WORKSPACE_ROOT"] = str(workspace_root)
        env["PYTHONMALLOC"] = "malloc"

        command = [
            "codspeed",
            "run",
            "--mode=simulation",
            "--skip-upload",
            f"--profile-folder={target_out_dir}",
            f"--working-directory={manifest_dir}",
            "--",
            str(bench_path),
        ]
        print(f"Measuring {target_key} ...", file=sys.stderr)
        subprocess.run(command, env=env, check=True)

        dumps = glob.glob(str(target_out_dir / "**" / "*.out"), recursive=True)
        if not dumps:
            raise SystemExit(f"No callgrind output produced for {target_key}")
        total_ir = 0
        for dump in dumps:
            try:
                total_ir += _parse_callgrind_ir(dump)
            except (subprocess.CalledProcessError, ValueError) as e:
                print(f"callgrind_annotate failed on {dump}: {e}", file=sys.stderr)
                if isinstance(e, subprocess.CalledProcessError):
                    print(e.stdout, file=sys.stderr)
                    print(e.stderr, file=sys.stderr)
                raise SystemExit(
                    f"{target_key}: could not get an instruction count from {dump}"
                ) from e
        if total_ir == 0:
            raise SystemExit(
                f"{target_key}: parsed an instruction count of 0 across "
                f"{len(dumps)} file(s) ({dumps}); a benchmark suite this "
                "size genuinely executing zero instructions is implausible."
            )
        results[target_key] = total_ir

    results_path = out_dir / "results.json"
    with open(results_path, "w", encoding="utf-8") as handle:
        json.dump(results, handle, indent=2, sort_keys=True)
    print(f"Wrote {results_path}")
    return results


def _load(path):
    with open(Path(path) / "results.json", encoding="utf-8") as handle:
        return json.load(handle)


def diff(base_dir, head_dir, md_path, json_path):
    base = _load(base_dir)
    head = _load(head_dir)
    targets = sorted(set(base) | set(head))

    rows = []
    any_regression = False
    for target in targets:
        base_ir = base.get(target)
        head_ir = head.get(target)
        if base_ir is None or head_ir is None:
            rows.append((target, base_ir, head_ir, None))
            continue
        pct = (head_ir - base_ir) / base_ir * 100 if base_ir else 0.0
        if pct > NOISE_FLOOR_PCT:
            any_regression = True
        rows.append((target, base_ir, head_ir, pct))

    lines = [
        "<!-- codspeed-local-diff -->",
        "### CodSpeed local diff (base vs. head, same runner)",
        "",
        "Measured with `codspeed run --skip-upload` (no upload) directly in "
        "this job, so base and head share the exact same CPU, glibc and "
        "kernel -- unlike a comparison against CodSpeed's own history, this "
        "diff cannot pick up a false regression from the two commits having "
        "drawn different runners. Counts are per `[[bench]]` target, not "
        "per individual benchmark, and are not comparable to CodSpeed's "
        "dashboard numbers.",
        "",
        "| Target | Base (Ir) | Head (Ir) | Change |",
        "| --- | ---: | ---: | ---: |",
    ]
    for target, base_ir, head_ir, pct in rows:
        if pct is None:
            lines.append(
                f"| `{target}` | {base_ir or '-'} | {head_ir or '-'} | new/removed |"
            )
            continue
        marker = (
            " :warning:"
            if pct > NOISE_FLOOR_PCT
            else (" :white_check_mark:" if pct < -NOISE_FLOOR_PCT else "")
        )
        lines.append(
            f"| `{target}` | {base_ir:,} | {head_ir:,} | {pct:+.2f}%{marker} |"
        )
    if any_regression:
        lines.append("")
        lines.append(f"At least one target grew by more than {NOISE_FLOOR_PCT}%.")

    Path(md_path).write_text("\n".join(lines) + "\n", encoding="utf-8")
    Path(json_path).write_text(
        json.dumps(
            {
                "targets": [
                    {"target": t, "base_ir": b, "head_ir": h, "pct_change": p}
                    for t, b, h, p in rows
                ],
                "any_regression": any_regression,
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    print("\n".join(lines))
    return any_regression


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subparsers = parser.add_subparsers(dest="command", required=True)

    measure_parser = subparsers.add_parser("measure")
    measure_parser.add_argument(
        "--out", required=True, help="directory to write results into"
    )
    measure_parser.add_argument(
        "--bench", help="only measure targets whose name contains this"
    )

    diff_parser = subparsers.add_parser("diff")
    diff_parser.add_argument(
        "--base", required=True, help="directory from a base `measure` run"
    )
    diff_parser.add_argument(
        "--head", required=True, help="directory from a head `measure` run"
    )
    diff_parser.add_argument(
        "--md", required=True, help="path to write the Markdown report to"
    )
    diff_parser.add_argument(
        "--json", required=True, help="path to write the JSON report to"
    )

    args = parser.parse_args(argv)

    if args.command == "measure":
        measure(args.out, args.bench)
    elif args.command == "diff":
        diff(args.base, args.head, args.md, args.json)

    return 0


if __name__ == "__main__":
    sys.exit(main())
