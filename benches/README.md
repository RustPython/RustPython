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
