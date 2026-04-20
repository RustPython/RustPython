#!/usr/bin/env python3
"""Test exception notes functionality in RustPython."""

print("Test 1: Basic exception notes")
try:
    e = ValueError("test error")
    e.add_note("This is a note")
    raise e
except ValueError as caught:
    print(f"Exception: {caught}")
    print(f"Has __notes__: {hasattr(caught, '__notes__')}")
    if hasattr(caught, '__notes__'):
        print(f"Notes: {caught.__notes__}")
print()

print("Test 2: Manual note addition to exception")
try:
    e = TypeError("original error")
    e.add_note("Error calling __set_name__ on '_proto_member' instance failed in 'BadSuper'")
    raise e
except TypeError as caught:
    print(f"Exception type: {type(caught).__name__}")
    print(f"Exception message: {caught}")
    if hasattr(caught, '__notes__'):
        print(f"Notes: {caught.__notes__}")
    else:
        print("No notes found")
print()

print("Test 3: Deleting __notes__")
try:
    e = ValueError("test")
    e.add_note("note 1")
    print(f"Before delete: {e.__notes__}")
    del e.__notes__
    print(f"After delete: has __notes__ = {hasattr(e, '__notes__')}")
except Exception as ex:
    print(f"Error: {ex}")
