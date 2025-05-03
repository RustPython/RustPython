# GitHub Copilot Instructions for RustPython

This document provides guidelines for working with GitHub Copilot when contributing to the RustPython project.

## Project Overview

RustPython is a Python 3 interpreter written in Rust, implementing Python 3.13.0+ compatibility. The project aims to provide:

- A complete Python-3 environment entirely in Rust (not CPython bindings)
- A clean implementation without compatibility hacks
- Cross-platform support, including WebAssembly compilation
- The ability to embed Python scripting in Rust applications

## Repository Structure

- `src/` - Top-level code for the RustPython binary
- `vm/` - The Python virtual machine implementation
  - `builtins/` - Python built-in types and functions
  - `stdlib/` - Essential standard library modules implemented in Rust, required to run the Python core
- `compiler/` - Python compiler components
  - `parser/` - Parser for converting Python source to AST
  - `core/` - Bytecode representation in Rust structures
  - `codegen/` - AST to bytecode compiler
- `Lib/` - CPython's standard library in Python (copied from CPython)
- `derive/` - Rust macros for RustPython
- `common/` - Common utilities
- `extra_tests/` - Integration tests and snippets
- `stdlib/` - Non-essential Python standard library modules implemented in Rust (useful but not required for core functionality)
- `wasm/` - WebAssembly support
- `jit/` - Experimental JIT compiler implementation
- `pylib/` - Python standard library packaging (do not modify this directory directly - its contents are generated automatically)

## Important Development Notes

### Running Python Code

When testing Python code, always use RustPython instead of the standard `python` command:

```bash
# Use this instead of python script.py
cargo run -- script.py

# For interactive REPL
cargo run

# With specific features
cargo run --features ssl

# Release mode (recommended for better performance)
cargo run --release -- script.py
```

### Comparing with CPython

When you need to compare behavior with CPython or run test suites:

```bash
# Use python command to explicitly run CPython
python my_test_script.py

# Run RustPython
cargo run -- my_test_script.py
```

### Working with the Lib Directory

The `Lib/` directory contains Python standard library files copied from the CPython repository. Important notes:

- These files should be edited very conservatively
- Modifications should be minimal and only to work around RustPython limitations
- Tests in `Lib/test` often use one of the following markers:
  - Add a `# TODO: RUSTPYTHON` comment when modifications are made
  - `unittest.skip("TODO: RustPython <reason>")`
  - `unittest.expectedFailure` with `# TODO: RUSTPYTHON <reason>` comment

### Testing

```bash
# Run Rust unit tests
cargo test --workspace --exclude rustpython_wasm

# Run Python snippets tests
cd extra_tests
pytest -v

# Run the Python test module
cargo run --release -- -m test
```

### Determining What to Implement

Run `./whats_left.py` to get a list of unimplemented methods, which is helpful when looking for contribution opportunities.

## Coding Guidelines

### Rust Code

- Follow the default rustfmt code style (`cargo fmt` to format)
- Use clippy to lint code (`cargo clippy`)
- Follow Rust best practices for error handling and memory management
- Use the macro system (`pyclass`, `pymodule`, `pyfunction`, etc.) when implementing Python functionality in Rust

### Python Code

- Follow PEP 8 style for custom Python code
- Use ruff for linting Python code
- Minimize modifications to CPython standard library files

## Integration Between Rust and Python

The project provides several mechanisms for integration:

- `pymodule` macro for creating Python modules in Rust
- `pyclass` macro for implementing Python classes in Rust
- `pyfunction` macro for exposing Rust functions to Python
- `PyObjectRef` and other types for working with Python objects in Rust

## Common Patterns

### Implementing a Python Module in Rust

```rust
#[pymodule]
mod mymodule {
    use rustpython_vm::prelude::*;

    #[pyfunction]
    fn my_function(value: i32) -> i32 {
        value * 2
    }

    #[pyattr]
    #[pyclass(name = "MyClass")]
    #[derive(Debug, PyPayload)]
    struct MyClass {
        value: usize,
    }

    #[pyclass]
    impl MyClass {
        #[pymethod]
        fn get_value(&self) -> usize {
            self.value
        }
    }
}
```

### Adding a Python Module to the Interpreter

```rust
vm.add_native_module(
    "my_module_name".to_owned(),
    Box::new(my_module::make_module),
);
```

## Building for Different Targets

### WebAssembly

```bash
# Build for WASM
cargo build --target wasm32-wasip1 --no-default-features --features freeze-stdlib,stdlib --release
```

### JIT Support

```bash
# Enable JIT support
cargo run --features jit
```

### SSL Support

```bash
# Enable SSL support
cargo run --features ssl
```

## Documentation

- Check the [architecture document](architecture/architecture.md) for a high-level overview
- Read the [development guide](DEVELOPMENT.md) for detailed setup instructions
- Generate documentation with `cargo doc --no-deps --all`
- Online documentation is available at [docs.rs/rustpython](https://docs.rs/rustpython/)