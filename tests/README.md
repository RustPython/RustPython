# Test snippets

This directory contains two sets of test snippets which can be run in Python.
The `snippets/` directory contains functional tests, and the `benchmarks/`
directory contains snippets for use in benchmarking RustPython's performance.

## Setup

Our testing depends on [pytest](https://pytest.org), which you can either
install globally using pip or locally using our
[pipenv](https://docs.pipenv.org).

## Running

Simply run `pytest` in this directory, and the tests should run (and hopefully
pass). If it hangs for a long time, that's because it's building RustPython in
release mode, which should take less time than it would to run every test
snippet with RustPython compiled in debug mode.
