# Benchmarking

These are some files to determine performance of rustpython.

## Usage

Install pytest and pytest-benchmark:

    $ pip install pytest-benchmark

Then run:

    $ pytest

You can also benchmark the Rust benchmarks by just running
`cargo bench` from the root of the repository. To view Python tracebacks during benchmarks,
run `RUST_BACKTRACE=1 cargo bench`.

You can bench against a specific Python version by running:

```shell
$ PYTHON_SYS_EXECUTABLE=python3.7 cargo bench
```

On MacOS you will need to
add the following to a `.cargo/config` file:

```toml
[target.x86_64-apple-darwin]
rustflags = [
    "-C", "link-arg=-undefined",
    "-C", "link-arg=dynamic_lookup",
]
```

## Benchmark source

- https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-python3-2.html
