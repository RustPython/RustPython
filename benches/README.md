# Benchmarking

These are some files to determine performance of rustpython.

## Usage

Running `cargo bench` from the root of the repository will start the benchmarks. Once done there will be a graphical 
report under `target/critierion/report/index.html` that you can use use to view the results.

To view Python tracebacks during benchmarks, run `RUST_BACKTRACE=1 cargo bench`. You can also bench against a 
specific installed Python version by running:

```shell
$ PYTHON_SYS_EXECUTABLE=python3.7 cargo bench
```

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

- https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-python3-2.html
