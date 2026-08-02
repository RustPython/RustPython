---
name: upgrade-pylib
description: Upgrade a Python standard library module from CPython into RustPython using scripts/update_lib and then triage and mark remaining failures.
allowed-tools: Bash(git add:*) Bash(git commit:*) Bash(git diff:*) Bash(cargo run:*) Bash(python3 scripts/update_lib quick:*) Bash(python3 scripts/update_lib auto-mark:*)
---

# Upgrade Python Library from CPython

Upgrade a Python standard library module from CPython to RustPython.

## Arguments

- Library name or path (examples: `inspect`, `asyncio`, `json`, `cpython/Lib/inspect.py`, `cpython/Lib/json/`)

## Important: Report Tool Issues First

If `scripts/update_lib` shows a bug, missing automation, or unexpected behavior, stop the upgrade and report the tooling issue first, including:

1. What you were trying to do:

- Library name.
- Full command used.

2. What went wrong or what is missing.
3. Expected vs actual behavior.

## Workflow

1. Run quick upgrade with update_lib:

- `python3 scripts/update_lib quick <module>`
- or `python3 scripts/update_lib quick cpython/Lib/<module>.py`
- or `python3 scripts/update_lib quick cpython/Lib/<module>/`

This step copies library files, patches tests while preserving markers where possible, runs tests, auto-marks new failures, removes stale `@unittest.expectedFailure`, and creates a commit.

If warnings like `WARNING: TestCFoo does not exist in remote file` appear, class structure changed and some markers must be restored manually.

2. Review diff and restore RustPython-specific changes:

- Run `git diff Lib/test/test_<module>`.
- Restore only changes with explicit `RUSTPYTHON` comments.
- Do not restore unrelated upstream CPython changes.

3. Investigate failing dependent tests:

- Get dependencies:

  ```shell
  cargo run --release -- scripts/update_lib deps <module>
  ```

- Find direct dependent test modules from the `- [ ] <module>:` line.
- Run dependent tests and collect failures:

  ```shell
  cargo run --release -- -m test <test_modules...> 2>&1 | grep -E "^(FAIL|ERROR):"
  ```

- For each failing test identifier, investigate with the `investigate-test-failure` workflow.

4. Mark remaining failures with auto-mark:

- `python3 scripts/update_lib auto-mark Lib/test/test_<module>.py --mark-failure`
- or `python3 scripts/update_lib auto-mark Lib/test/test_<module>/ --mark-failure`

Note: `--mark-failure` marks all current failures including regressions; review carefully.

5. Handle panics manually:

- For tests that panic/crash/hang, use `@unittest.skip("TODO: RUSTPYTHON; ...")` rather than expectedFailure.

6. Handle class-specific failures:

- If only one subclass fails (for example `TestCFoo` but not `TestPyFoo`), move marker to the failing subclass override.

7. Commit test-fix markers as a separate commit:

```shell
git add -u && git commit -m "Mark failing tests"
```

## Example usage

```text
upgrade-pylib inspect
upgrade-pylib json
upgrade-pylib asyncio
upgrade-pylib cpython/Lib/inspect.py
upgrade-pylib cpython/Lib/json/
```

## Notes

- The `cpython/` directory should contain the CPython source used for sync.
- `scripts/update_lib` modes:
  - `quick`: patch + auto-mark
  - `migrate`: patch only
  - `auto-mark`: mark failures only
  - `copy-lib`: copy library files only
- Patching transfers `@unittest.expectedFailure` and `@unittest.skip` with `TODO: RUSTPYTHON` markers where possible.
- The tool does not preserve every RustPython-specific change; always review and restore explicit RUSTPYTHON-marked logic.
