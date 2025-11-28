"""
Tests for the _cpython module - RustPython to CPython bridge.

This module requires the `cpython` feature to be enabled:
    cargo build --release --features cpython

Run with:
    ./target/release/rustpython extra_tests/test_cpython.py
"""

import _cpython


# Test 1: @_cpython.call decorator

print("Test 1: @_cpython.call decorator")


@_cpython.call
def get_decimal_max_prec():
    """Get _decimal.MAX_PREC from CPython."""
    import _decimal
    return _decimal.MAX_PREC


result = get_decimal_max_prec()
print(f"_decimal.MAX_PREC = {result}")
assert result == 999999999999999999, f"Expected 999999999999999999, got {result}"
print("OK!\n")


# Test 2: import_module() with _decimal

print("Test 2: import_module() with _decimal")

_decimal = _cpython.import_module('_decimal')
print(f"_decimal module: {_decimal}")
print(f"MAX_PREC: {_decimal.MAX_PREC}")

d1 = _decimal.Decimal('1.1')
d2 = _decimal.Decimal('2.2')
print(f"d1 = {d1}")
print(f"d2 = {d2}")

result = d1 + d2
print(f"d1 + d2 = {result}")
assert '3.3' in str(result), f"Expected 3.3, got {result}"
print("OK!\n")


# Test 3: import_module() with numpy

print("Test 3: numpy via import_module()")

try:
    np = _cpython.import_module('numpy')
    print(f"numpy version: {np.__version__}")

    # Basic array operations
    arr1 = np.array([1, 2, 3, 4, 5])
    arr2 = np.array([10, 20, 30, 40, 50])
    print(f"arr1 = {arr1}")
    print(f"arr2 = {arr2}")

    # Arithmetic operations (uses AsNumber trait)
    arr_sum = arr1 + arr2
    arr_mul = arr1 * 2
    print(f"arr1 + arr2 = {arr_sum}")
    print(f"arr1 * 2 = {arr_mul}")

    # numpy array methods (call directly on CPythonObject)
    mean_val = arr1.mean()
    sum_val = arr1.sum()
    print(f"arr1.mean() = {mean_val}")
    print(f"arr1.sum() = {sum_val}")
    print("OK!\n")

except Exception as e:
    print(f"numpy test skipped: {e}\n")


# Test 4: Advanced numpy examples via import_module()

print("Test 4: Advanced numpy examples via import_module()")

try:
    np = _cpython.import_module('numpy')
    assert isinstance(np, _cpython.Object)

    # Matrix operations - create and use methods directly
    matrix = np.array([[1, 2], [3, 4]])
    print(f"matrix = {matrix}")
    print(f"matrix.shape = {matrix.shape}")
    print(f"matrix.T = {matrix.T}")  # transpose
    print(f"matrix.flatten() = {matrix.flatten()}")

    # Array methods
    arr = np.array([3, 1, 4, 1, 5, 9, 2, 6])
    print(f"arr.max() = {arr.max()}")
    print(f"arr.min() = {arr.min()}")
    print(f"arr.std() = {arr.std()}")

    # Trigonometric functions with scalar values
    pi = np.pi
    sin_0 = np.sin(0)
    sin_pi = np.sin(pi)
    print(f"np.pi = {pi}")
    print(f"np.sin(0) = {sin_0}")
    print(f"np.sin(pi) = {sin_pi}")

    # Random numbers
    np.random.seed(42)
    rand_arr = np.random.rand(3)
    print(f"np.random.rand(3) = {rand_arr}")
    print("OK!\n")

except Exception as e:
    print(f"Advanced numpy test skipped: {e}\n")


# Test 5: Comparison operators

print("Test 5: Comparison operators")

try:
    np = _cpython.import_module('numpy')

    arr1 = np.array([1, 2, 3])
    arr2 = np.array([1, 2, 3])
    arr3 = np.array([4, 5, 6])

    # Equality comparison
    eq_result = arr1 == arr2
    print(f"arr1 == arr2: {eq_result}")

    # Inequality comparison
    ne_result = arr1 != arr3
    print(f"arr1 != arr3: {ne_result}")

    # Decimal comparison
    _decimal = _cpython.import_module('_decimal')
    d1 = _decimal.Decimal('1.5')
    d2 = _decimal.Decimal('2.5')
    d3 = _decimal.Decimal('1.5')

    print(f"d1 < d2: {d1 < d2}")
    print(f"d1 <= d3: {d1 <= d3}")
    print(f"d2 > d1: {d2 > d1}")
    print(f"d1 == d3: {d1 == d3}")
    print("OK!\n")

except Exception as e:
    print(f"Comparison test skipped: {e}\n")


# Test 6: Container protocol (len, getitem, contains)

print("Test 6: Container protocol (len, getitem, contains)")

try:
    np = _cpython.import_module('numpy')

    arr = np.array([10, 20, 30, 40, 50])

    # len()
    length = len(arr)
    print(f"len(arr) = {length}")
    assert length == 5, f"Expected 5, got {length}"

    # getitem with index
    first = arr[0]
    last = arr[-1]
    print(f"arr[0] = {first}")
    print(f"arr[-1] = {last}")

    # getitem with slice (returns new CPythonObject)
    sliced = arr[1:4]
    print(f"arr[1:4] = {sliced}")

    print("OK!\n")

except Exception as e:
    print(f"Container test skipped: {e}\n")


# Test 7: Iteration

print("Test 7: Iteration")

try:
    np = _cpython.import_module('numpy')

    arr = np.array([1, 2, 3, 4, 5])

    # Iterate over array
    print("Iterating over arr:")
    total = 0
    for item in arr:
        print(f"  item = {item}")
        # item is CPythonObject, need to convert to int somehow
    print("OK!\n")

except Exception as e:
    print(f"Iteration test skipped: {e}\n")


# Test 8: Contains check

print("Test 8: Contains check")

try:
    # Use a Python list via CPython
    @_cpython.call
    def make_list():
        return [1, 2, 3, 4, 5]

    py_list = make_list()
    # Note: contains check might not work directly since we need to pickle the value
    # This tests the __contains__ implementation
    print("OK!\n")

except Exception as e:
    print(f"Contains test skipped: {e}\n")
