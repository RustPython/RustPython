# Running upstream pyperformance against RustPython

[pyperformance](https://github.com/python/pyperformance) cannot be installed
as-is on RustPython: its dependency `pyperf` hard-depends on `psutil`, a
C-extension package, and RustPython has no CPython C-API / extension-module
loading support (`_imp.create_dynamic` / `_imp.exec_dynamic` are not
implemented, and `_sysconfigdata` doesn't provide a full compiler config, so
even building a C extension from source fails).

Workaround: `pyperf` already disables psutil usage on interpreters that
report `Py_GIL_DISABLED=1` in `sysconfig` (see `pyperf._utils.USE_PSUTIL`) --
this is exactly what RustPython reports, since it has no GIL. So `pyperf`
never actually calls into psutil at runtime on RustPython; a
functionality-free pure-Python `psutil` stub (see `stub_psutil/`) is enough
to satisfy pip's dependency resolution and let installation proceed.

This only unblocks *pure-Python* benchmarks. Any benchmark whose own
workload needs a real C extension (numpy, lxml, greenlet, PyYAML's C
accelerator, ...) will still fail to install -- and that failure is a real,
separate finding about RustPython's lack of C-extension support, not an
artifact of this workaround.

## Usage

```shell
# Build RustPython first (SSL feature needed for --install-pip, not required here)
cargo build --release

# Run every benchmark pyperformance knows about against it, and write a catalog
python3 scripts/pyperformance/run_all.py --rustpython target/release/rustpython

# Or just a subset
python3 scripts/pyperformance/run_all.py --benchmarks nqueens,richards,pyflate
```

Output goes to `scripts/pyperformance/results/`:
- `CATALOG.md` -- human-readable pass/fail table with the mean time and a
  failure reason for each benchmark.
- `catalog.json` -- the same data, machine-readable.
- `raw/<benchmark>.json` -- pyperf's own result file for benchmarks that
  passed.

The run resumes automatically: re-running the script skips any benchmark
already present in `catalog.json`. Pass `--force` to redo everything.

A host CPython venv (pyperformance itself only runs under a real CPython;
RustPython is just the `--python` target it benchmarks) and the stub wheel
are cached under `target/pyperformance/` (gitignored) and reused across
runs.

See `python3 scripts/pyperformance/run_all.py --help` for all options
(`--timeout`, `--rigorous` instead of `--fast`, `--cache-dir`, ...).
