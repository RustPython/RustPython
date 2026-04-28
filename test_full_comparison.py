#!/usr/bin/env python3
"""Full comparison of CPython vs RustPython behavior for test_bad_new_super."""

import sys
from enum import Enum

print("=" * 70)
print(f"Python implementation: {sys.implementation.name}")
print(f"Python version: {sys.version}")
print("=" * 70)
print()

print("Creating BadSuper enum with custom __new__ that uses super().__new__")
print()

exception_raised = None
exception_type = None
exception_message = None
exception_cause = None
exception_notes = None

try:
    class BadSuper(Enum):
        def __new__(cls, value):
            obj = super().__new__(cls, value)
            return obj
        failed = 1
    print("ERROR: No exception was raised!")
except Exception as e:
    exception_raised = e
    exception_type = type(e).__name__
    exception_message = str(e)
    exception_cause = e.__cause__ if hasattr(e, '__cause__') else None
    exception_notes = e.__notes__ if hasattr(e, '__notes__') else None

print("RESULTS:")
print("-" * 70)
print(f"Exception raised: {exception_raised is not None}")
if exception_raised:
    print(f"Exception type: {exception_type}")
    print(f"Exception message: {exception_message}")
    print(f"Has __cause__: {exception_cause is not None}")
    if exception_cause:
        print(f"  __cause__ type: {type(exception_cause).__name__}")
        print(f"  __cause__ message: {exception_cause}")
    print(f"Has __notes__: {exception_notes is not None}")
    if exception_notes:
        print(f"  __notes__: {exception_notes}")
print("-" * 70)
print()

print("EXPECTED BEHAVIOR (CPython 3.12+):")
print("  - Exception type: TypeError")
print("  - Exception message: do not use `super().__new__`; call the appropriate __new__ directly")
print("  - __cause__: None (or at least not wrapped in RuntimeError)")
print("  - __notes__: None (deleted by enum.py lines 555-556)")
print()

print("TEST VERDICT:")
if exception_type == "TypeError" and "super().__new__" in exception_message:
    print("  ✓ PASS: Correct exception type and message")
else:
    print(f"  ✗ FAIL: Expected TypeError with 'super().__new__' message")
    print(f"         Got {exception_type}: {exception_message}")
