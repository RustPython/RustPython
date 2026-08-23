#!/usr/bin/env python3
"""PR performance regression gate for RustPython.

Measures retired instruction counts (callgrind "Ir") for a fixed set of
Python workloads executed by a RustPython binary, and compares two such
measurements (base vs head) with a relative threshold.

Why instruction counts instead of wall-clock time?

* GitHub Actions runners are shared, throttled machines; wall-clock numbers
  routinely swing by 10-40% between runs, which forces either huge thresholds
  (misses real regressions) or a flaky gate (noise blocks unrelated PRs).
* Instruction counts from callgrind are deterministic for a deterministic
  program: repeated runs of the same binary+workload differ by well under
  0.1% (see benches/perf_ci/README.md for measured numbers), and are immune
  to CPU contention, frequency scaling, and co-tenant noise.
* Determinism also means results are comparable when base and head are
  measured in parallel processes, which keeps the job inside its time budget.

Instruction count is a proxy for time (it cannot see cache/branch effects),
but for an interpreter hot-loop it tracks real cost closely and, above all,
it makes the gate reproducible: a red gate is always caused by the diff.

The interpreter is run with PYTHONHASHSEED=0 so that str/bytes hashing, and
therefore dict layout, is identical across runs.

Usage:
  scripts/perf_ci.py list
  scripts/perf_ci.py measure --binary target/release/rustpython -o head.json
  scripts/perf_ci.py compare base.json head.json --threshold 0.02
"""

import argparse
import concurrent.futures
import json
import math
import os
import subprocess
import sys
import tempfile
import time

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PERF_DIR = os.path.join("benches", "perf_ci")
MICRO = os.path.join(PERF_DIR, "micro")
PYPERF = os.path.join(PERF_DIR, "pyperformance")

# Workload table. Each entry: name -> (argv after the binary, extra env).
# Sizes are tuned so that a single run takes roughly 50-500 ms natively
# (a few seconds under callgrind) while still executing enough guest code
# that interpreter startup (~135M Ir) stays a small fraction of the total.
WORKLOADS = {
    # Interpreter startup cost: process creation to exit with a no-op program.
    "startup": (["-c", "pass"], {}),
    # Import machinery: startup plus a handful of common stdlib imports.
    "import_stdlib": ([os.path.join(MICRO, "import_stdlib.py")], {}),
    # Targeted microbenchmarks, one interpreter axis each.
    "int_arith": ([os.path.join(MICRO, "int_arith.py")], {}),
    "float_arith": ([os.path.join(MICRO, "float_arith.py")], {}),
    "call_function": ([os.path.join(MICRO, "call_function.py")], {}),
    "method_call": ([os.path.join(MICRO, "method_call.py")], {}),
    "attr_load": ([os.path.join(MICRO, "attr_load.py")], {}),
    "dict_ops": ([os.path.join(MICRO, "dict_ops.py")], {}),
    "list_ops": ([os.path.join(MICRO, "list_ops.py")], {}),
    "str_ops": ([os.path.join(MICRO, "str_ops.py")], {}),
    "class_create": ([os.path.join(MICRO, "class_create.py")], {}),
    "exceptions": ([os.path.join(MICRO, "exceptions.py")], {}),
    # Vendored pyperformance kernels (see benches/perf_ci/pyperformance/).
    # Workload sizes are shrunk from upstream defaults via CLI args or env so
    # each run stays affordable under callgrind's ~50x slowdown.
    "nbody": ([os.path.join(PYPERF, "bm_nbody.py"), "--iterations", "500"], {}),
    "chaos": (
        [
            os.path.join(PYPERF, "bm_chaos.py"),
            "--iterations",
            "500",
            "--width",
            "128",
            "--height",
            "128",
        ],
        {},
    ),
    "raytrace": (
        [os.path.join(PYPERF, "bm_raytrace.py"), "--width", "24", "--height", "24"],
        {},
    ),
    "deltablue": (
        [os.path.join(PYPERF, "bm_deltablue.py")],
        {"PERF_CI_DELTABLUE_N": "30"},
    ),
    "float": ([os.path.join(PYPERF, "bm_float.py")], {"PERF_CI_FLOAT_POINTS": "20000"}),
    "nqueens": (
        [os.path.join(PYPERF, "bm_nqueens.py")],
        {"PERF_CI_NQUEENS_COUNT": "7"},
    ),
    "fannkuch": (
        [os.path.join(PYPERF, "bm_fannkuch.py")],
        {"PERF_CI_FANNKUCH_ARG": "8"},
    ),
    "spectral_norm": (
        [os.path.join(PYPERF, "bm_spectral_norm.py")],
        {"PERF_CI_SPECTRAL_NORM_N": "60"},
    ),
    "richards": ([os.path.join(PYPERF, "bm_richards.py")], {}),
    "scimark_sor": (
        [os.path.join(PYPERF, "bm_scimark.py"), "sor"],
        {"PERF_CI_SCIMARK_SOR_N": "40"},
    ),
    "scimark_monte_carlo": (
        [os.path.join(PYPERF, "bm_scimark.py"), "monte_carlo"],
        {"PERF_CI_SCIMARK_MONTE_CARLO_N": "20000"},
    ),
}

DEFAULT_THRESHOLD = 0.02


def measure_one(binary, name, out_dir):
    argv, extra_env = WORKLOADS[name]
    out_file = os.path.join(out_dir, "callgrind.%s.out" % name)
    env = dict(os.environ)
    env["PYTHONHASHSEED"] = "0"
    env.setdefault("RUSTPYTHONPATH", os.path.join(REPO_ROOT, "Lib"))
    env.update(extra_env)
    cmd = [
        "valgrind",
        "--tool=callgrind",
        "--callgrind-out-file=%s" % out_file,
        "--quiet",
        binary,
    ] + argv
    start = time.monotonic()
    proc = subprocess.run(cmd, cwd=REPO_ROOT, env=env, capture_output=True, text=True)
    elapsed = time.monotonic() - start
    if proc.returncode != 0:
        raise RuntimeError(
            "workload %r failed (exit %d):\n%s\n%s"
            % (name, proc.returncode, proc.stdout[-2000:], proc.stderr[-2000:])
        )
    ir = parse_ir(out_file)
    os.unlink(out_file)
    return name, ir, elapsed


def parse_ir(out_file):
    events = None
    with open(out_file) as f:
        for line in f:
            if line.startswith("events:"):
                events = line.split()[1:]
            elif line.startswith("summary:"):
                values = [int(v) for v in line.split()[1:]]
                if events is None or "Ir" not in events:
                    raise RuntimeError("no Ir event in %s" % out_file)
                return values[events.index("Ir")]
    raise RuntimeError("no summary line in %s" % out_file)


def cmd_measure(args):
    binary = os.path.abspath(args.binary)
    names = args.bench or sorted(WORKLOADS)
    unknown = set(names) - set(WORKLOADS)
    if unknown:
        sys.exit("unknown workloads: %s" % ", ".join(sorted(unknown)))
    jobs = args.jobs or max(1, (os.cpu_count() or 2) - 1)
    results = {}
    wall = time.monotonic()
    with tempfile.TemporaryDirectory() as out_dir:
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            futures = [
                pool.submit(measure_one, binary, name, out_dir) for name in names
            ]
            for fut in concurrent.futures.as_completed(futures):
                name, ir, elapsed = fut.result()
                results[name] = ir
                print(
                    "%-22s %14s Ir  (%.1fs under callgrind)"
                    % (name, format(ir, ","), elapsed),
                    flush=True,
                )
    wall = time.monotonic() - wall
    payload = {
        "binary": binary,
        "unit": "instructions (callgrind Ir)",
        "results": results,
    }
    with open(args.output, "w") as f:
        json.dump(payload, f, indent=2, sort_keys=True)
        f.write("\n")
    print("measured %d workloads in %.0fs -> %s" % (len(results), wall, args.output))


def cmd_compare(args):
    with open(args.base) as f:
        base = json.load(f)["results"]
    with open(args.head) as f:
        head = json.load(f)["results"]

    common = sorted(set(base) & set(head))
    if not common:
        sys.exit("no common workloads between %s and %s" % (args.base, args.head))
    only_base = sorted(set(base) - set(head))
    only_head = sorted(set(head) - set(base))

    rows = []
    regressions = []
    for name in common:
        delta = (head[name] - base[name]) / base[name]
        status = "ok"
        if delta > args.threshold:
            status = "REGRESSION"
            regressions.append((name, delta))
        elif delta < -args.threshold:
            status = "improved"
        rows.append((name, base[name], head[name], delta, status))

    geomean = (
        math.exp(sum(math.log(head[n] / base[n]) for n in common) / len(common)) - 1.0
    )

    lines = []
    lines.append("| workload | base Ir | head Ir | delta | status |")
    lines.append("|---|---:|---:|---:|---|")
    for name, b, h, delta, status in rows:
        mark = {"REGRESSION": "❌", "improved": "✅", "ok": ""}[status]
        lines.append(
            "| %s | %s | %s | %+.2f%% | %s %s |"
            % (name, format(b, ","), format(h, ","), delta * 100, mark, status)
        )
    lines.append("")
    lines.append(
        "Geometric mean delta: **%+.2f%%** (threshold per workload: +%.1f%%)"
        % (geomean * 100, args.threshold * 100)
    )
    for name in only_base:
        lines.append("- `%s` only present in base measurement" % name)
    for name in only_head:
        lines.append("- `%s` only present in head measurement" % name)
    report = "\n".join(lines)

    print(report)
    summary_path = args.summary or os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a") as f:
            f.write("## Performance gate (callgrind instruction counts)\n\n")
            f.write(report)
            f.write("\n")

    if regressions:
        print()
        print(
            "FAIL: %d workload(s) regressed more than %.1f%%:"
            % (len(regressions), args.threshold * 100)
        )
        for name, delta in regressions:
            print("  %-22s %+.2f%%" % (name, delta * 100))
        sys.exit(1)
    print()
    print("PASS: no workload regressed more than %.1f%%" % (args.threshold * 100))


def cmd_list(_args):
    for name in sorted(WORKLOADS):
        argv, env = WORKLOADS[name]
        extra = " ".join("%s=%s" % kv for kv in sorted(env.items()))
        print("%-22s %s %s" % (name, " ".join(argv), extra))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("measure", help="measure Ir for each workload")
    p.add_argument("--binary", required=True, help="rustpython binary to measure")
    p.add_argument("-o", "--output", required=True, help="output JSON path")
    p.add_argument(
        "--bench",
        action="append",
        help="measure only this workload (repeatable; default: all)",
    )
    p.add_argument(
        "--jobs",
        type=int,
        help="parallel callgrind processes (default: cpu_count - 1); "
        "instruction counts are unaffected by concurrency",
    )
    p.set_defaults(func=cmd_measure)

    p = sub.add_parser("compare", help="compare two measurement files")
    p.add_argument("base", help="JSON produced by `measure` for the base commit")
    p.add_argument("head", help="JSON produced by `measure` for the head commit")
    p.add_argument(
        "--threshold",
        type=float,
        default=DEFAULT_THRESHOLD,
        help="max allowed relative Ir increase per workload (default: %(default)s)",
    )
    p.add_argument(
        "--summary",
        help="markdown summary output path (default: $GITHUB_STEP_SUMMARY)",
    )
    p.set_defaults(func=cmd_compare)

    p = sub.add_parser("list", help="list workloads")
    p.set_defaults(func=cmd_list)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
