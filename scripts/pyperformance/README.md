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

## Comparing against real CPython

`run_all.py` can target any Python executable, not just RustPython, via
`--python` + `--label`. For a real CPython you don't need (and don't want)
the psutil stub -- pass `--no-psutil-stub` so it installs and uses the
genuine `psutil`:

```shell
python3 scripts/pyperformance/run_all.py \
    --python "$(command -v python3.13)" --label cpython3.13 --no-psutil-stub

python3 scripts/pyperformance/run_all.py \
    --python target/release/rustpython --label rustpython
```

Each `--label` gets its own subdirectory under `results/` (its own
`catalog.json`, `CATALOG.md`, `raw/`), so multiple targets' catalogs coexist.

Then compare two catalogs:

```shell
python3 scripts/pyperformance/compare.py --baseline cpython3.13 --candidate rustpython
```

This writes `results/COMPARE-rustpython-vs-cpython3.13.md`: a per-benchmark
table with both targets' status/mean and the candidate/baseline time ratio,
plus the median slowdown across benchmarks both targets passed.

Note: pass an actual interpreter binary to `--python`, not a version-manager
shim (e.g. an `asdf`/`mise`/`pyenv` shim) -- pyperformance runs a helper
script through it directly, which some shims don't support. If unsure what a
`python3` on your PATH really resolves to, use
`python3 -c "import sys; print(sys.executable)"` and pass that path.
