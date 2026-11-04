# Benchmarking

These are some files to determine performance of rustpython.

## Usage

Running `cargo bench` from the root of the repository will start the benchmarks. Once done there will be a graphical
report under `target/criterion/report/index.html` that you can use use to view the results.

`cargo bench` supports name matching to run a subset of the benchmarks. To
run only the sorted microbenchmark, you can run:

```shell
cargo bench sorted
```

To view Python tracebacks during benchmarks, run `RUST_BACKTRACE=1 cargo bench`. You can also bench against a
specific installed Python version by running:

```shell
PYTHON_SYS_EXECUTABLE=python3.13 cargo bench
```

## Continuous benchmarking with CodSpeed

RustPython's performance is tracked on every pull request by
[CodSpeed](https://app.codspeed.io/RustPython/RustPython) in wall-time mode on a CodSpeed Macro
runner (dedicated hardware). The workflow in `.github/workflows/codspeed.yaml` builds the
benchmark executables on a hosted runner and moves only those to the Macro runner. `criterion` is
aliased to `codspeed-criterion-compat`, so nothing changes for `cargo bench`: the compatibility layer
only takes over when the CodSpeed runner drives the benchmarks.

CodSpeed measures two targets:

- `pyperformance` (`benches/pyperformance.rs`): the upstream pyperformance benchmark programs,
  vendored unmodified under `benches/pyperformance/`. See the README there for how they run and
  which ones are included.
- the `rustpython-sre_engine` benchmarks.

The `execution` and `microbenchmarks` targets below are not run by CodSpeed. They link CPython
through `pyo3` for the comparison, and an executable linked against one machine's `libpython`
cannot be moved to another; they remain local tools.

To reproduce a CodSpeed run locally, install [`cargo-codspeed`](https://crates.io/crates/cargo-codspeed)
(same major version as `codspeed-criterion-compat` in `Cargo.toml`) and the
[CodSpeed CLI](https://codspeed.io/docs/cli), then run:

```shell
cargo codspeed build --measurement-mode walltime -p rustpython --bench pyperformance -p rustpython-sre_engine
codspeed run --mode walltime -- cargo codspeed run -p rustpython --bench pyperformance -p rustpython-sre_engine
```

### Adding a benchmark

Simply adding a file to the `benchmarks/` directory will add it to the set of files benchmarked. Each file is tested
in two ways:

1. The time to parse the file to AST
2. The time it takes to execute the file

Parsing stops at an AST in both implementations. The CPython reference uses
`compile(..., flags=ast.PyCF_ONLY_AST)` without compiling bytecode.

Execution includes compilation and runs each iteration with a fresh namespace whose `__name__`
is `"__main__"`, so script entry points run in both RustPython and CPython.

Both Rust harnesses load only `.py` files and run workloads in filename order.
Other entries, such as `__pycache__` directories, are ignored.

### Adding a micro benchmark

Micro benchmarks are small snippets of code added under the `microbenchmarks/` directory. A microbenchmark file has
two sections:

1. Optional setup code
2. The code to be benchmarked

These two sections are delimited by `# ---`. For example:

```python
a_list = [1,2,3]

# ---

len(a_list)
```

Only `len(a_list)` will be timed. Setup or benchmarked code can optionally reference a variable called `ITERATIONS`. If
present then the benchmark runs at five sizes: 100, 300, 500, 700 and 900 iterations locally. For
example:

```python
obj = [i for i in range(ITERATIONS)]
```

`ITERATIONS` can appear in both the setup code and the benchmark code.

## Subprocess benchmarks with pytest

The optional pytest driver measures whole interpreter processes, including startup.
Install `pytest` and `pytest-benchmark`, build RustPython with `cargo build --release`,
and run from the repository root:

```shell
python -m pytest benches/test_benchmarks.py
```

It uses the release executable in `target/release` by default. Set `RUSTPYTHON` to
the executable path when using a different build directory. Relative `RUSTPYTHON`
paths are resolved from the directory where pytest is launched; absolute paths
can be used from any directory. Script paths are resolved relative to this
checkout, so pytest can also be launched from another directory. A failed
subprocess fails the benchmark instead of producing a timing.

## MacOS setup

On MacOS you will need to add the following to a `.cargo/config` file:

```toml
[target.x86_64-apple-darwin]
rustflags = [
    "-C", "link-arg=-undefined",
    "-C", "link-arg=dynamic_lookup",
]
```

## Benchmark source

- <https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-python3-2.html>
- The pyperformance programs under `pyperformance/` come from
  [python/pyperformance 1.14.0](https://github.com/python/pyperformance/tree/1.14.0/pyperformance/data-files/benchmarks)
  and are not modified; see `pyperformance/README.md`.
