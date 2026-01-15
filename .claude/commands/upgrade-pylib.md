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

3. **Upgrade tests**
   - Run: `python lib_updater.py --quick-upgrade cpython/Lib/test/test_$ARGUMENTS`
   - This will update the test files with appropriate RustPython markers

## Example Usage
```
/upgrade-pylib inspect
/upgrade-pylib json
/upgrade-pylib asyncio
```

## Notes
- The cpython/ directory should contain the CPython source that we're syncing from
- lib_updater.py handles adding `# TODO: RUSTPYTHON` markers and `@unittest.expectedFailure` decorators
- After upgrading, you may need to run tests to verify: `cargo run --release -- -m test test_$ARGUMENTS`
