# AI Agent Instructions for RustPython

RustPython is a Python 3 interpreter written in Rust, not a CPython binding. It targets Python 3.15, with compatibility work in progress. Prefer a clean implementation over compatibility hacks.

See [CONTRIBUTING.md](CONTRIBUTING.md#code-organization) for the directory layout and [architecture/architecture.md](architecture/architecture.md) for a high-level overview.

## AI Agent Rules

**CRITICAL: AI Policy**

- Follow RustPython's [AI Policy](https://github.com/RustPython/.github/blob/main/AI_POLICY.md) for every AI-assisted contribution.
- Disclose AI assistance in commit messages with an `Assisted-by: AGENT_NAME:MODEL_VERSION` trailer. Use one trailer per AI tool, and never use `Co-authored-by` for an AI assistant.

**CRITICAL: Git Operations**

- NEVER push commits or create pull requests without explicit user permission.
- Commits can be created locally when requested.

**CRITICAL: Commit Hooks**

- Install the repository's pre-commit hook with `prek install` (or `pre-commit install`) after cloning the repository.
- Every commit must run the configured pre-commit hook. NEVER bypass it with `--no-verify`. Automated workflows that use a normal `git commit`, such as `scripts/update_lib quick`, should be allowed to create local commits through the hook.
- If a hook auto-fixes files (e.g. `ruff-format`, `rustfmt`), re-stage the fixes and retry the commit. Do not amend or force a failing commit through.

## Testing

Run Python code with RustPython (`cargo run -- script.py`). `python` is CPython; use it only to compare behavior.

Before completing a task, run the checks that match the change:

| Change | Required checks |
| --- | --- |
| Rust code | Workspace tests, C-API tests, and Clippy |
| `Lib/` or interpreter behavior | The above, plus the relevant `-m test <module>` modules |
| `extra_tests/snippets/` | `pytest -v` from `extra_tests` |
| Documentation only | None |

```bash
# Rust unit tests
cargo test --workspace --exclude rustpython_wasm --exclude rustpython-venvlauncher --exclude rustpython-capi

# C-API tests, from their directory so their separate Cargo config applies
(cd crates/capi && cargo test)

# Clippy, with the same C-API separation
cargo clippy --workspace --all-targets --exclude rustpython_wasm --exclude rustpython-venvlauncher --exclude rustpython-capi
(cd crates/capi && cargo clippy --all-targets)

# A Python test module, or one test in it (release mode: faster to run)
cargo run --release -- -m test test_unicode
cargo run --release -- -m test test_unicode -k test_unicode_escape

# One snippet (debug mode: faster to compile), or all of them
cargo run -- extra_tests/snippets/builtin_bytes.py
(cd extra_tests && pytest -v)
```

- Never add tests to `extra_tests/snippets` that duplicate coverage in `Lib/test`.
- Prefer adding tests to the most closely related existing file in `extra_tests/snippets` rather than creating a new snippet file.
- When you modify bytecode instructions, a full clean is required:

  ```bash
  rm -r target/debug/build/rustpython-* && find . -name '*.pyc' -delete
  ```

## Working with the Lib Directory

The `Lib/` directory contains Python standard library files copied from the CPython repository.

- Fix bugs in Rust code. In most cases, files under `Lib/` should not be edited.
- When an edit is unavoidable, keep it minimal, limit it to working around a RustPython limitation, and mark it with a `# TODO: RUSTPYTHON` comment.
- See [How to update test files](https://github.com/RustPython/RustPython/wiki/How-to-update-test-files#checkout-cpython-source-code-initial-setup) for syncing files from upstream CPython.

### Test Files in `Lib/test`

**CRITICAL**: The only acceptable modifications to the CPython test files in `Lib/test` are:

1. Adding a marker to a test that fails on RustPython and cannot be fixed now
2. Removing a marker, and its TODO comment, from a test that now passes

NEVER modify test assertions, test logic, or test data. NEVER comment out or delete test code.

Markers take one of these forms:

```python
@unittest.expectedFailure  # TODO: RUSTPYTHON; <reason>
@unittest.expectedFailureIf(<condition>, "TODO: RUSTPYTHON; <reason>")
@unittest.skip("TODO: RUSTPYTHON; <reason>")
@unittest.skipIf(<condition>, "TODO: RUSTPYTHON; <reason>")
```

- Prefer `expectedFailure` / `expectedFailureIf`. The test body still runs, so a later fix surfaces as an unexpected pass. Use the `*If` form when the failure is environment-specific (e.g., a platform or build flag).
- Use `skip` / `skipIf` only when running the test would take down the test process: segfaults, Rust panics, aborts, or hangs.

## Rust Code

- Run `cargo fmt` and Clippy before completing a task. Fix any warnings introduced by your changes.
- When branches differ only in a value but share common logic, extract the differing value first, then call the common logic once.
- Do not delete or rewrite existing comments unless they are factually wrong or directly contradict the new code.
- Do not add decorative section separators (e.g. `// -----------`, `// ===`, `/* *** */`).
- A `///` doc comment on a Python-exposed item (`#[pyattr]`, `#[pyclass]`, `#[pyfunction]`, `#[pymethod]`, `#[pyclassmethod]`, `#[pystaticmethod]`, `#[pygetset]`, `#[pymember]`, `#[pyslot]`, `#[pyexception]`, `#[pymodule]`, `#[pystruct_sequence]`) is that item's docstring and overrides the stored docs in `rustpython-doc` (`crates/doc`). Those stored docs are used only when the item has no `///` comment. Developer notes on these items must be `//` comments.

## Other Notes

- **WebAssembly**: `cargo build --target wasm32-wasip1 --no-default-features --features freeze-stdlib,stdlib --release`
- **Linux build and debug on macOS**: see [CONTRIBUTING.md](CONTRIBUTING.md#testing-on-linux-from-macos).
- **venvlauncher (Windows)**: all 4 venvlauncher binaries use the same source code. Do NOT add multiple `[[bin]]` entries to Cargo.toml. Build once and copy with different names.
- **CI workflows**: a change to any file under `.github/workflows/` must pass a [zizmor](https://docs.zizmor.sh/) scan in CI.
- **`./scripts/whats_left.py`**: lists unimplemented methods. It runs under CPython 3.15 or newer, not RustPython.
