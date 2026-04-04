# PyO3 embed against RustPython C-API

This example demonstrates linking `pyo3` against RustPython's minimal C-API shim (`rustpython-capi`) instead of a system CPython library.

From this directory, run:

```shell
cargo run
```

The local `.cargo/config.toml` sets `PYO3_CONFIG_FILE` automatically.
