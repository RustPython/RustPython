# PR performance regression gate workloads

These workloads back the `Performance gate` workflow
(`.github/workflows/perf-ci.yaml`), which blocks pull requests that regress
interpreter performance. They are driven by `scripts/perf_ci.py`, which runs
each file under `valgrind --tool=callgrind` and compares retired instruction
counts (Ir) between the PR head and its merge base.

## Why instruction counts

Wall-clock timings on shared CI runners are noisy (10-40% run-to-run swings),
which makes any wall-clock threshold either too loose or flaky. Instruction
counts of a deterministic program are reproducible to well under 0.1%
(`PYTHONHASHSEED=0` is pinned so dict/str hashing is deterministic), are
immune to CPU contention and frequency scaling, and remain comparable when
measurements run in parallel. Ir cannot observe cache or branch-predictor
effects, so the gate can miss (rare) regressions that change memory locality
without changing instruction count — the scheduled criterion benchmarks
(`cron-ci.yaml`) still track wall-clock trends over time for that.

Role split:

* `cron-ci.yaml` `benchmark` job — scheduled criterion wall-clock runs,
  published to the website; long-term trend data, never blocks PRs.
* This gate — per-PR, deterministic, blocks regressions.

Two runs of the gate share a checkout, so bytecode caching has to be handled
explicitly: RustPython writes `.pyc` files next to the sources it imports, and
loading a cached module is far cheaper than compiling it (measured: ~3.8x
fewer instructions for `import_stdlib`, ~2x for `chaos`). Whichever binary ran
second would otherwise inherit the first one's compiled bytecode and look up
to 46% faster with no interpreter change at all. `scripts/perf_ci.py`
therefore purges the cache at the start of every measurement run and warms it
with the binary it is about to measure, then measures with `-B` so the timed
runs cannot mutate it. Both binaries are thus compared in the same steady
state, and the numbers do not depend on measurement order.

## Layout

* `micro/` — targeted microbenchmarks written for this gate, one interpreter
  axis per file: integer/float arithmetic, function calls, method calls,
  instance attribute loads, dict/list/str operations, class creation,
  exception handling, and stdlib import cost. Interpreter startup (`-c pass`)
  is measured directly by `scripts/perf_ci.py`.
* `pyperformance/` — benchmark kernels vendored from
  [pyperformance](https://github.com/python/pyperformance) 1.14.0 (MIT
  license, see `pyperformance/COPYING`). Modifications are limited to
  `# RUSTPYTHON perf_ci` blocks that let the gate shrink workload sizes via
  environment variables (upstream defaults are kept).
  `pyperformance/pyperf.py` is a minimal local stand-in for
  the real pyperf harness so the kernels run unmodified; it is our code, not
  vendored.

The full pyperformance harness is not used because it hard-requires building
`psutil` (a CPython C extension) into a venv managed by the measured
interpreter, which RustPython cannot currently do. Wall-clock statistics from
pyperf would also not fit the deterministic instruction-count approach.

## Running locally

Drive the harness with CPython 3.14 (the version CONTRIBUTING.md requires and
the one CI uses); the script itself only needs the stdlib, plus `valgrind` on
the system. All workloads also run unmodified under CPython, which is handy
for sanity-checking a workload change.

```shell
cargo build --release
python3 scripts/perf_ci.py measure --binary target/release/rustpython -o head.json
git stash        # or check out the base commit and rebuild
python3 scripts/perf_ci.py measure --binary target/release/rustpython -o base.json
python3 scripts/perf_ci.py compare base.json head.json
```

Workload sizes are tuned so one full measurement of one binary takes a few
minutes (callgrind slows execution ~50x). When adding a workload, keep its
native runtime in the 50-500 ms range and make it deterministic: fixed seeds,
no wall-clock dependence, no filesystem or network I/O in the hot path.
