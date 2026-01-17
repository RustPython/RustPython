# Upgrade Python Library from CPython

Upgrade a Python standard library module from CPython to RustPython.

## Arguments
- `$ARGUMENTS`: Library name to upgrade (e.g., `inspect`, `asyncio`, `json`)

## Steps

1. **Delete existing library in Lib/**
   - If `Lib/$ARGUMENTS.py` exists, delete it
   - If `Lib/$ARGUMENTS/` directory exists, delete it

2. **Copy from cpython/Lib/**
   - If `cpython/Lib/$ARGUMENTS.py` exists, copy it to `Lib/$ARGUMENTS.py`
   - If `cpython/Lib/$ARGUMENTS/` directory exists, copy it to `Lib/$ARGUMENTS/`

3. **Upgrade tests (quick upgrade with lib_updater)**
   - If `cpython/Lib/test/test_$ARGUMENTS.py` is a single file:
     - Run: `python3 scripts/lib_updater.py --quick-upgrade cpython/Lib/test/test_$ARGUMENTS.py`
   - If `cpython/Lib/test/test_$ARGUMENTS/` is a directory:
     - Run the script for each `.py` file in the directory:
       ```bash
       for f in cpython/Lib/test/test_$ARGUMENTS/*.py; do
           python3 scripts/lib_updater.py --quick-upgrade "$f"
       done
       ```
   - This will update the test files with basic RustPython markers (`@unittest.expectedFailure`, `@unittest.skip`, etc.)
   - **Handle lib_updater warnings**: If you see warnings like `WARNING: TestCFoo does not exist in remote file`, it means the class structure changed between versions and markers couldn't be transferred automatically. These need to be manually restored in step 4 or added in step 5.

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
- `scripts/lib_updater.py` handles basic patching:
  - Transfers `@unittest.expectedFailure` and `@unittest.skip` decorators with `TODO: RUSTPYTHON` markers
  - Adds `import unittest # XXX: RUSTPYTHON` if needed for the decorators
  - **Limitation**: If a class was restructured (e.g., method overrides removed), lib_updater will warn and skip those markers
- The script does NOT preserve all RustPython-specific changes - you must review `git diff` and restore them
- Common RustPython markers to look for:
  - `# XXX: RUSTPYTHON` or `# XXX RUSTPYTHON` - Inline comments for code modifications
  - `# TODO: RUSTPYTHON` - Test skip/failure markers
  - Any code with `RUSTPYTHON` in comments that was removed in the diff
- **Important**: Not all changes in the git diff need to be restored. Only restore changes that have explicit `RUSTPYTHON` comments. Other changes are upstream CPython updates.
