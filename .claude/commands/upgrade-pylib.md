# Upgrade Python Library from CPython

Upgrade a Python standard library module from CPython to RustPython.

## Arguments
- `$ARGUMENTS`: Library name to upgrade (e.g., `inspect`, `asyncio`, `json`)

## Important: Report Tool Issues First

If during the upgrade process you encounter any of the following issues with `scripts/update_lib`:
- A feature that should be automated but isn't supported
- A bug or unexpected behavior in the tool
- Missing functionality that would make the upgrade easier

**STOP the upgrade and report the issue first.** Describe:
1. What you were trying to do
  - Library name
  - The full command executed (e.g. python scripts/update_lib quick cpython/Lib/$ARGUMENTS.py)
2. What went wrong or what's missing
3. Expected vs actual behavior

This helps improve the tooling for future upgrades.

## Steps

1. **Delete existing library in Lib/**
   - If `Lib/$ARGUMENTS.py` exists, delete it
   - If `Lib/$ARGUMENTS/` directory exists, delete it

2. **Copy from cpython/Lib/**
   - If `cpython/Lib/$ARGUMENTS.py` exists, copy it to `Lib/$ARGUMENTS.py`
   - If `cpython/Lib/$ARGUMENTS/` directory exists, copy it to `Lib/$ARGUMENTS/`

3. **Upgrade tests (quick upgrade with update_lib)**
   - Run: `python3 scripts/update_lib quick cpython/Lib/test/test_$ARGUMENTS.py` (single file)
   - Or: `python3 scripts/update_lib quick cpython/Lib/test/test_$ARGUMENTS/` (directory)
   - This will:
     - Patch test files preserving existing RustPython markers
     - Run tests and auto-mark new test failures (not regressions)
     - Remove `@unittest.expectedFailure` from tests that now pass
   - **Handle warnings**: If you see warnings like `WARNING: TestCFoo does not exist in remote file`, it means the class structure changed and markers couldn't be transferred automatically. These need to be manually restored in step 4 or added in step 5.

4. **Review git diff and restore RUSTPYTHON-specific changes**
   - Run `git diff Lib/test/test_$ARGUMENTS` to review all changes
   - **Only restore changes that have explicit `RUSTPYTHON` comments**. Look for:
     - `# XXX: RUSTPYTHON` or `# XXX RUSTPYTHON` - Comments marking RustPython-specific code modifications
     - `# TODO: RUSTPYTHON` - Comments marking tests that need work
     - Code changes with inline `# ... RUSTPYTHON` comments
   - **Do NOT restore other diff changes** - these are likely upstream CPython changes, not RustPython-specific modifications
   - When restoring, preserve the original context and formatting

5. **Verify tests**
   - Run: `cargo run --release -- -m test test_$ARGUMENTS -v`
   - The `-v` flag shows detailed output to identify which tests fail and why
   - For each new failure, add appropriate markers based on the failure type:
     - **Test assertion failure** → `@unittest.expectedFailure` with `# TODO: RUSTPYTHON` comment
     - **Panic/crash** → `@unittest.skip("TODO: RUSTPYTHON; <panic message>")`
   - **Class-specific markers**: If a test fails only in the C implementation (TestCFoo) but passes in the Python implementation (TestPyFoo), or vice versa, add the marker to the specific subclass, not the base class:
     ```python
     # Base class - no marker here
     class TestFoo:
         def test_something(self):
             ...

     class TestPyFoo(TestFoo, PyTest): pass

     class TestCFoo(TestFoo, CTest):
         # TODO: RUSTPYTHON
         @unittest.expectedFailure
         def test_something(self):
             return super().test_something()
     ```
   - **New tests from CPython**: The upgrade may bring in entirely new tests that didn't exist before. These won't have any RUSTPYTHON markers in the diff - they just need to be tested and marked if they fail.
   - Example markers:
     ```python
     # TODO: RUSTPYTHON
     @unittest.expectedFailure
     def test_something(self):
         ...

     # TODO: RUSTPYTHON
     @unittest.skip("TODO: RUSTPYTHON; panics with 'index out of bounds'")
     def test_crashes(self):
         ...
     ```

## Example Usage
```
/upgrade-pylib inspect
/upgrade-pylib json
/upgrade-pylib asyncio
```

## Example: Restoring RUSTPYTHON changes

When git diff shows removed RUSTPYTHON-specific code like:
```diff
-# XXX RUSTPYTHON: we don't import _json as fresh since...
-cjson = import_helper.import_fresh_module('json') #, fresh=['_json'])
+cjson = import_helper.import_fresh_module('json', fresh=['_json'])
```

You should restore the RustPython version:
```python
# XXX RUSTPYTHON: we don't import _json as fresh since...
cjson = import_helper.import_fresh_module('json') #, fresh=['_json'])
```

## Notes
- The cpython/ directory should contain the CPython source that we're syncing from
- `scripts/update_lib` package handles patching and auto-marking:
  - `quick` - Combined patch + auto-mark (recommended)
  - `migrate` - Only migrate (patch), no test running
  - `auto-mark` - Only run tests and mark failures
  - `copy-lib` - Copy library files (not tests)
- The patching:
  - Transfers `@unittest.expectedFailure` and `@unittest.skip` decorators with `TODO: RUSTPYTHON` markers
  - Adds `import unittest # XXX: RUSTPYTHON` if needed for the decorators
  - **Limitation**: If a class was restructured (e.g., method overrides removed), update_lib will warn and skip those markers
- The smart auto-mark:
  - Marks NEW test failures automatically (tests that didn't exist before)
  - Does NOT mark regressions (existing tests that now fail) - these are warnings
  - Removes `@unittest.expectedFailure` from tests that now pass
- The script does NOT preserve all RustPython-specific changes - you must review `git diff` and restore them
- Common RustPython markers to look for:
  - `# XXX: RUSTPYTHON` or `# XXX RUSTPYTHON` - Inline comments for code modifications
  - `# TODO: RUSTPYTHON` - Test skip/failure markers
  - Any code with `RUSTPYTHON` in comments that was removed in the diff
- **Important**: Not all changes in the git diff need to be restored. Only restore changes that have explicit `RUSTPYTHON` comments. Other changes are upstream CPython updates.
