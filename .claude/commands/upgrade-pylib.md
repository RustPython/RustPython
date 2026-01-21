---
allowed-tools: Bash(git add:*), Bash(git commit:*), Bash(python3 scripts/update_lib quick:*), Bash(python3 scripts/update_lib auto-mark:*)
---

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

1. **Run quick upgrade with update_lib**
   - Run: `python3 scripts/update_lib quick $ARGUMENTS` (module name)
   - Or: `python3 scripts/update_lib quick cpython/Lib/$ARGUMENTS.py` (library file path)
   - Or: `python3 scripts/update_lib quick cpython/Lib/$ARGUMENTS/` (library directory path)
   - This will:
     - Copy library files (delete existing `Lib/$ARGUMENTS.py` or `Lib/$ARGUMENTS/`, then copy from `cpython/Lib/`)
     - Patch test files preserving existing RustPython markers
     - Run tests and auto-mark new test failures (not regressions)
     - Remove `@unittest.expectedFailure` from tests that now pass
     - Create a git commit with the changes
   - **Handle warnings**: If you see warnings like `WARNING: TestCFoo does not exist in remote file`, it means the class structure changed and markers couldn't be transferred automatically. These need to be manually restored in step 2 or added in step 3.

2. **Review git diff and restore RUSTPYTHON-specific changes**
   - Run `git diff Lib/test/test_$ARGUMENTS` to review all changes
   - **Only restore changes that have explicit `RUSTPYTHON` comments**. Look for:
     - `# XXX: RUSTPYTHON` or `# XXX RUSTPYTHON` - Comments marking RustPython-specific code modifications
     - `# TODO: RUSTPYTHON` - Comments marking tests that need work
     - Code changes with inline `# ... RUSTPYTHON` comments
   - **Do NOT restore other diff changes** - these are likely upstream CPython changes, not RustPython-specific modifications
   - When restoring, preserve the original context and formatting

3. **Mark remaining test failures with auto-mark**
   - Run: `python3 scripts/update_lib auto-mark Lib/test/test_$ARGUMENTS.py --mark-failure`
   - Or for directory: `python3 scripts/update_lib auto-mark Lib/test/test_$ARGUMENTS/ --mark-failure`
   - This will:
     - Run tests and mark ALL failing tests with `@unittest.expectedFailure`
     - Remove `@unittest.expectedFailure` from tests that now pass
   - **Note**: The `--mark-failure` flag marks all failures including regressions. Review the changes before committing.

4. **Handle panics manually**
   - If any tests cause panics/crashes (not just assertion failures), they need `@unittest.skip` instead:
     ```python
     @unittest.skip("TODO: RUSTPYTHON; panics with 'index out of bounds'")
     def test_crashes(self):
         ...
     ```
   - auto-mark cannot detect panics automatically - check the test output for crash messages

5. **Handle class-specific failures**
   - If a test fails only in the C implementation (TestCFoo) but passes in the Python implementation (TestPyFoo), or vice versa, move the marker to the specific subclass:
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

6. **Commit the test fixes**
   - Run: `git add -u && git commit -m "Mark failing tests"`
   - This creates a separate commit for the test markers added in steps 2-5

## Example Usage
```
# Using module names (recommended)
/upgrade-pylib inspect
/upgrade-pylib json
/upgrade-pylib asyncio

# Using library paths (alternative)
/upgrade-pylib cpython/Lib/inspect.py
/upgrade-pylib cpython/Lib/json/
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
