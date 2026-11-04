# Instructions for coding agents

These instructions apply to work in this repository. Follow the user's request first and use only guidance relevant to the files and behavior being changed. Do not turn optional workflows into requirements for unrelated tasks. If instructions conflict, follow the higher-priority instruction and explain any consequential limitation.

## Working approach

1. Read the relevant code and nearby tests before editing. Check `git status` and preserve existing user changes.
2. Trace the behavior to its implementation and follow established patterns in the closest related code. Prefer the smallest complete change; avoid unrelated cleanup and speculative refactors.
3. Make tests reflect the behavior being changed. Run focused checks first, then broader checks when the change warrants them. Report exactly what ran and what did not; do not claim unrun checks passed.
4. Summarize the change and any remaining limitation in the final response. Do not commit, push, or open a pull request unless the user asks.

If the request is clear, proceed without asking for confirmation. Ask only when a missing decision materially changes the implementation and cannot be resolved from the repository or conversation.

## Project context

RustPython is a Python interpreter implemented in Rust, targeting Python 3.15 with compatibility work in progress. It supports native embedding and WebAssembly. See [CONTRIBUTING.md](CONTRIBUTING.md#code-organization) for repository layout and [architecture/architecture.md](architecture/architecture.md) for design context when relevant.

Prefer implementing interpreter behavior in Rust. Python sources under `Lib/` are synchronized from CPython and should be changed sparingly, for RustPython-specific limitations only. Custom Python code should follow PEP 8 and be linted with Ruff when appropriate.

Run Python code and tests with RustPython. Use CPython only when explicitly comparing behavior, and compare the same input on both interpreters.

## Tests and compatibility

- Prefer a focused test near the code being changed. Do not add snippet tests that duplicate coverage in `Lib/test`; extend the closest existing `extra_tests/snippets` file when suitable.
- For Rust changes, run the relevant crate's tests. For interpreter-wide changes, use the workspace test command below when practical. Run C-API tests separately from `crates/capi` because it has its own Cargo configuration.
- For changes to `Lib/` or Python-visible interpreter behavior, run the relevant module with `cargo run --release -- -m test <module>` when practical. For changes under `extra_tests/snippets/`, run the relevant snippet or `pytest -v` from `extra_tests` when practical.
- Run `cargo fmt` for Rust formatting changes and the relevant Clippy command for Rust code changes. Fix warnings introduced by the change.
- Do not edit CPython test assertions, logic, or data; do not comment out or delete test code. In `Lib/test`, the permitted changes are adding a RustPython-specific expected failure or removing an existing expected-failure marker and its associated TODO when the test now passes. Document failures with `TODO: RUSTPYTHON; <reason>`; use `expectedFailureIf` when the limitation is environment-specific. Use `skip`/`skipIf` only if executing the test would crash or hang the process; otherwise prefer `expectedFailure` so a future fix is visible. Find existing markers with `rg 'TODO: RUSTPYTHON' Lib/test`.
- If a test requires unsupported syntax or another missing feature, leave it expected-failing and explain the limitation; fix the implementation when feasible rather than weakening the test.

Verification commands (choose based on scope; do not run all of them for every change):

```bash
# Workspace Rust tests, excluding targets with separate or platform-specific setup
cargo test --workspace --exclude rustpython_wasm --exclude rustpython-venvlauncher --exclude rustpython-capi

# C-API tests, from the crate directory
(cd crates/capi && cargo test)

# Workspace Clippy
cargo clippy --workspace --all-targets --exclude rustpython_wasm --exclude rustpython-venvlauncher --exclude rustpython-capi
(cd crates/capi && cargo clippy --all-targets)

# A Python test module or one test within it
cargo run --release -- -m test <module>
cargo run --release -- -m test <module> -k <test_name>

# A snippet; use pytest from extra_tests for the full snippet suite
cargo run -- <snippet.py>
(cd extra_tests && pytest -v)
```

When changing bytecode instructions, perform a clean rebuild: remove `target/debug/build/rustpython-*` and generated `.pyc` files after verifying the paths.

For Python modules implemented in Rust, follow nearby `pymodule` and `pyclass` patterns, import from `rustpython_vm::prelude`, expose functions and methods with `pyfunction` and `pymethod`, and register native modules through the VM API. Use `PyObjectRef` and related VM types for Python objects.

## Rust conventions

- Use standard `rustfmt` formatting and established RustPython patterns, including `pyclass`, `pymodule`, and related macros for Python-facing APIs.
- Do not remove or rewrite existing comments unless they are wrong or contradicted by the change. Avoid decorative comment separators.
- A `///` doc comment on a Python-exposed item becomes its Python docstring and overrides documentation stored in `crates/doc`. Use `//` for implementation notes on such items.
- When branches share logic and differ only in a value, compute the value first and call the shared logic once.

## Git and contribution policy

- Follow RustPython's [AI Policy](https://github.com/RustPython/.github/blob/main/AI_POLICY.md) for AI-assisted contributions.
- If creating a commit, include an `Assisted-by: AGENT_NAME:MODEL_VERSION` trailer for each AI tool used. Do not use `Co-authored-by` for an AI assistant.
- Local commits are allowed only when requested. Never push or create a pull request without explicit user authorization. Every commit must run the configured hook: do not use `--no-verify`; if it fixes files, stage the fixes and retry normally.
- After cloning, install the configured pre-commit hook with `prek install` or `pre-commit install` before committing.
- If modifying `.github/workflows/`, ensure the workflow passes a [zizmor](https://docs.zizmor.sh/) scan.
- All four Windows `venvlauncher` binaries use the same source. Build once and copy under the required names; do not add extra `[[bin]]` entries.

## Useful references

- [Development guide](CONTRIBUTING.md)
- [Testing on Linux from macOS](CONTRIBUTING.md#testing-on-linux-from-macos)
- [Updating CPython test files](https://github.com/RustPython/RustPython/wiki/How-to-update-test-files#checkout-cpython-source-code-initial-setup)
- [API documentation](https://docs.rs/rustpython/)
- `./scripts/whats_left.py` lists unimplemented methods when looking for implementation gaps.
- WASI builds use the `wasm32-wasip1` target with `freeze-stdlib` and `stdlib`; JIT support is enabled by the `jit` feature.
- Linux builds and debugging on macOS are described in the development guide.
- Generate project documentation with `cargo doc --no-deps --all` when documentation changes warrant it.
