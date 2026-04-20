#!/usr/bin/env python3
"""Minimal test to reproduce test_bad_new_super failure."""

from enum import Enum

print("Testing BadSuper enum creation...")
print("Expected: TypeError with message 'do not use `super().__new__`'")
print()

try:
    class BadSuper(Enum):
        def __new__(cls, value):
            obj = super().__new__(cls, value)
            return obj
        failed = 1
    print(f"ERROR: No exception raised! Created enum: {BadSuper}")
except TypeError as e:
    print(f"SUCCESS: Got TypeError: {e}")
    if hasattr(e, '__notes__'):
        print(f"  Notes: {e.__notes__}")
except RuntimeError as e:
    print(f"FAIL: Got RuntimeError instead of TypeError: {e}")
    if hasattr(e, '__notes__'):
        print(f"  Notes: {e.__notes__}")
    if hasattr(e, '__cause__'):
        print(f"  Cause: {e.__cause__}")
except Exception as e:
    print(f"FAIL: Got unexpected exception {type(e).__name__}: {e}")
    if hasattr(e, '__notes__'):
        print(f"  Notes: {e.__notes__}")
    if hasattr(e, '__cause__'):
        print(f"  Cause: {e.__cause__}")
