# GitHub Copilot Instructions for RustPython

This document provides guidelines for working with GitHub Copilot when contributing to the RustPython project.

## Project Overview

RustPython is a Python 3 interpreter written in Rust, implementing Python 3.14.0+ compatibility. The project aims to provide:

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
- `Lib/` - CPython's standard library in Python (copied from CPython). **IMPORTANT**: Do not edit this directory directly; The only allowed operation is copying files from CPython.
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
cargo run --features jit

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

### Clean Build

When you modify bytecode instructions, a full clean is required:

```bash
rm -r target/debug/build/rustpython-* && find . | grep -E "\.pyc$" | xargs rm -r
```

### Testing

```bash
# Run Rust unit tests
cargo test --workspace --exclude rustpython_wasm

# Run Python snippets tests (debug mode recommended for faster compilation)
cargo run -- extra_tests/snippets/builtin_bytes.py

# Run all Python snippets tests with pytest
cd extra_tests
pytest -v

# Run the Python test module (release mode recommended for better performance)
cargo run --release -- -m test ${TEST_MODULE}
cargo run --release -- -m test test_unicode # to test test_unicode.py

# Run the Python test module with specific function
cargo run --release -- -m test test_unicode -k test_unicode_escape
```

**Note**: For `extra_tests/snippets` tests, use debug mode (`cargo run`) as compilation is faster. For `unittest` (`-m test`), use release mode (`cargo run --release`) for better runtime performance.

### Determining What to Implement

Run `./scripts/whats_left.py` to get a list of unimplemented methods, which is helpful when looking for contribution opportunities.

## Coding Guidelines

### Rust Code

- Follow the default rustfmt code style (`cargo fmt` to format)
- **IMPORTANT**: Always run clippy to lint code (`cargo clippy`) before completing tasks. Fix any warnings or lints that are introduced by your changes
- Follow Rust best practices for error handling and memory management
- Use the macro system (`pyclass`, `pymodule`, `pyfunction`, etc.) when implementing Python functionality in Rust

### Python Code

- **IMPORTANT**: In most cases, Python code should not be edited. Bug fixes should be made through Rust code modifications only
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

### Building venvlauncher (Windows)

See DEVELOPMENT.md "CPython Version Upgrade Checklist" section.

**IMPORTANT**: All 4 venvlauncher binaries use the same source code. Do NOT add multiple `[[bin]]` entries to Cargo.toml. Build once and copy with different names.

## Test Code Modification Rules

**CRITICAL: Test code modification restrictions**
- NEVER comment out or delete any test code lines except for removing `@unittest.expectedFailure` decorators and upper TODO comments
- NEVER modify test assertions, test logic, or test data
- When a test cannot pass due to missing language features, keep it as expectedFailure and document the reason
- The only acceptable modifications to test files are:
  1. Removing `@unittest.expectedFailure` decorators and the upper TODO comments when tests actually pass
  2. Adding `@unittest.expectedFailure` decorators when tests cannot be fixed

**Examples of FORBIDDEN modifications:**
- Commenting out test lines
- Changing test assertions
- Modifying test data or expected results
- Removing test logic

**Correct approach when tests fail due to unsupported syntax:**
- Keep the test as `@unittest.expectedFailure`
- Document that it requires PEP 695 support
- Focus on tests that can be fixed through Rust code changes only

## Documentation

- Check the [architecture document](/architecture/architecture.md) for a high-level overview
- Read the [development guide](/DEVELOPMENT.md) for detailed setup instructions
- Generate documentation with `cargo doc --no-deps --all`
- Online documentation is available at [docs.rs/rustpython](https://docs.rs/rustpython/)
