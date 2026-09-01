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

The benchmarks are also run on every pull request by
[CodSpeed](https://app.codspeed.io/RustPython/RustPython), which measures them with CPU simulation
instead of wall time. `criterion` is aliased to `codspeed-criterion-compat`, so nothing changes for
`cargo bench`: the compatibility layer only takes over when the CodSpeed runner drives the
benchmarks.

To reproduce a CodSpeed run locally, install [`cargo-codspeed`](https://crates.io/crates/cargo-codspeed)
and the [CodSpeed CLI](https://codspeed.io/docs/cli), then run:

```shell
cargo codspeed build -p rustpython -p rustpython-sre_engine
codspeed run --mode simulation -- cargo codspeed run -p rustpython -p rustpython-sre_engine
```

Two things differ when the benchmarks run under CodSpeed:

- The CPython comparison benchmarks are skipped. They are useful to compare RustPython against
  CPython locally, but CodSpeed tracks the evolution of RustPython itself, and running them would
  double the duration of an already slow instrumented run.
- The microbenchmarks using `ITERATIONS` run with a single value instead of five. The criterion
  benchmark id does not include the iteration count, so all five sizes are reported under the same
  name.

### Adding a benchmark

Simply adding a file to the `benchmarks/` directory will add it to the set of files benchmarked. Each file is tested
in two ways:

1. The time to parse the file to AST
2. The time it takes to execute the file

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
present then the benchmark code will be invoked 5 times with `ITERATIONS` set to a value between 100 and 1,000. For
example:

```python
obj = [i for i in range(ITERATIONS)]
```

`ITERATIONS` can appear in both the setup code and the benchmark code.

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
- The workloads marked as adapted from pyperformance come from
  [python/pyperformance 1.14.0](https://github.com/python/pyperformance/tree/1.14.0/pyperformance/data-files/benchmarks).
  Their `pyperf` runners and internal timers are removed so Criterion is the only measurement
  harness, and some input sizes are reduced for CodSpeed simulation.
