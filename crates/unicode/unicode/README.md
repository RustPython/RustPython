# Unicode data

The vendored data files come directly from the official [Unicode site](https://www.unicode.org/).

The files in `latest` need to be periodically bumped to match `icu4x` and Rust. These files may be found at the [Unicode Character Database](https://www.unicode.org/ucd/).

RustPython vendors [Unicode 3.2.0](https://www.unicode.org/reports/tr28/tr28-3.html) to match CPython. CPython uses 3.2.0 to ensure backwards compatibility with a few older modules. Unicode 3.2.0 was released in 2002 and does not need periodic refreshes.
