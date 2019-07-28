# Benchmarking

These are some files to determine performance of rustpython.

## Usage

Install pytest and pytest-benchmark:

    $ pip install pytest-benchmark

Then run:

    $ pytest

You can also benchmark the Rust benchmarks by just running
`cargo +nightly bench` from the root of the repository. Make sure you have Rust
nightly installed, as the benchmarking parts of the standard library are still
unstable.

## Benchmark source

- https://benchmarksgame-team.pages.debian.net/benchmarksgame/program/nbody-python3-2.html
