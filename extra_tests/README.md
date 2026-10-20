# Test snippets

This directory contains two sets of test snippets which can be run in Python.
The `snippets/` directory contains functional tests, and the `benchmarks/`
directory contains snippets for use in benchmarking RustPython's performance.

## Setup

Our testing depends on [pytest](https://pytest.org), which you can install
using pip.

## Running

Simply run `pytest -v` in this directory, and the tests should run (and hopefully
pass). If it hangs for a long time, that's because it's building RustPython in
release mode, which should take less time than it would to run every test
snippet with RustPython compiled in debug mode.

## Regression result collector

`jsontests.py` uses the canonical regrtest process setup before discovery. Its
JSON and tables count each parent test once; failed subtests make their parent
fail. The standard unittest summary counts individual failure/error/skip events
and can therefore differ. Each case retains its full `id` and all subtest
callbacks reported by unittest, including their IDs, outcomes and tracebacks.
Historical result files are not rewritten.
