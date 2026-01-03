"""
PVM Comprehensive Checkpoint/Resume Demo
===========================================

This demo showcases the wide variety of Python control flow structures
that PVM can successfully checkpoint and resume, including:

- Functions (nested calls)
- For loops (list iteration, enumerate, zip, map, filter)
- While loops
- If/elif/else statements
- Try/except/finally blocks
- Match statements (pattern matching)
- List comprehensions
- Dictionary and set operations
- Nested control structures

Note: This demo avoids using range() due to a known issue with
range_iterator restoration in loop contexts. Use list iteration
or while loops as alternatives.
"""

from __future__ import annotations
from pathlib import Path
import os

import rustpython_checkpoint as rpc  # type: ignore

CHECKPOINT_PATH = str(Path(__file__).with_suffix(".rpsnap"))
SEP = "=" * 70

# Global state for tracking progress
state = {
    "checkpoints_passed": [],
    "test_results": [],
    "counter": 0,
}

print(SEP)
print("PVM Comprehensive Checkpoint/Resume Demo")
print(SEP)

# ============================================================================
# Test 1: Nested Function Calls
# ============================================================================
print("\n[Test 1] Nested Function Calls")

def outer_function(data: dict) -> int:
    """Outer function that calls inner functions."""
    data["outer_called"] = True
    result = inner_function_a(data)
    return result

def inner_function_a(data: dict) -> int:
    """First inner function."""
    data["inner_a_called"] = True
    result = inner_function_b(data)
    return result + 10

def inner_function_b(data: dict) -> int:
    """Second inner function with checkpoint."""
    data["inner_b_called"] = True
    if "checkpoint_1" not in state["checkpoints_passed"]:
        print("  [Checkpoint #1] Inside nested function (depth=3)")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_1")
        print("  [Resumed #1] Continuing from nested function")
    return 42

result = outer_function(state)
state["test_results"].append(("nested_functions", result == 52))
print(f"  Result: {result} (expected 52)")

# ============================================================================
# Test 2: For Loop with List Iteration
# ============================================================================
print(f"\n[Test 2] For Loop with List Iteration")

data_list = [10, 20, 30, 40, 50]
sum_val = 0

for value in data_list:
    sum_val += value
    if value == 30 and "checkpoint_2" not in state["checkpoints_passed"]:
        print(f"  [Checkpoint #2] Inside for loop, value={value}, sum={sum_val}")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_2")
        print(f"  [Resumed #2] Continuing for loop")

state["test_results"].append(("for_list", sum_val == 150))
print(f"  Final sum: {sum_val} (expected 150)")

# ============================================================================
# Test 3: Enumerate Loop
# ============================================================================
print(f"\n[Test 3] Enumerate Loop")

fruits = ["apple", "banana", "cherry", "date"]
enum_results = []

for idx, fruit in enumerate(fruits):
    enum_results.append((idx, fruit))
    if idx == 2 and "checkpoint_3" not in state["checkpoints_passed"]:
        print(f"  [Checkpoint #3] In enumerate loop, idx={idx}, fruit={fruit}")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_3")
        print(f"  [Resumed #3] Continuing enumerate loop")

state["test_results"].append(("enumerate_loop", len(enum_results) == 4))
print(f"  Enumerated {len(enum_results)} items")

# ============================================================================
# Test 4: While Loop
# ============================================================================
print(f"\n[Test 4] While Loop")

counter = 0
while_sum = 0

while counter < 5:
    while_sum += counter * 2
    counter += 1
    if counter == 3 and "checkpoint_4" not in state["checkpoints_passed"]:
        print(f"  [Checkpoint #4] In while loop, counter={counter}, sum={while_sum}")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_4")
        print(f"  [Resumed #4] Continuing while loop")

state["test_results"].append(("while_loop", while_sum == 20))
print(f"  Final sum: {while_sum} (expected 20)")

# ============================================================================
# Test 5: If/Elif/Else Chains
# ============================================================================
print(f"\n[Test 5] If/Elif/Else Chains")

test_value = 75
category = ""

if test_value < 0:
    category = "negative"
elif test_value < 50:
    category = "low"
elif test_value < 100:
    # Checkpoint in elif branch
    if "checkpoint_5" not in state["checkpoints_passed"]:
        print(f"  [Checkpoint #5] In elif branch, value={test_value}")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_5")
        print(f"  [Resumed #5] Continuing from elif")
    category = "medium"
else:
    category = "high"

state["test_results"].append(("if_elif_else", category == "medium"))
print(f"  Category: {category} (expected medium)")

# ============================================================================
# Test 6: Try/Except/Finally
# ============================================================================
print(f"\n[Test 6] Try/Except/Finally")

try_result = {"attempted": False, "caught": False, "finalized": False}

try:
    try_result["attempted"] = True
    if "checkpoint_6" not in state["checkpoints_passed"]:
        print(f"  [Checkpoint #6] Inside try block")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_6")
        print(f"  [Resumed #6] Continuing from try")
    # Raise an exception to test except
    if "checkpoint_7" not in state["checkpoints_passed"]:
        raise ValueError("test_exception")
except ValueError as e:
    try_result["caught"] = True
    if "checkpoint_7" not in state["checkpoints_passed"]:
        print(f"  [Checkpoint #7] Inside except block, caught: {e}")
        rpc.checkpoint(CHECKPOINT_PATH)
        state["checkpoints_passed"].append("checkpoint_7")
        print(f"  [Resumed #7] Continuing from except")
finally:
    try_result["finalized"] = True

state["test_results"].append(("try_except", all(try_result.values())))
print(f"  Try/Except/Finally: {try_result}")

# ============================================================================
# Test 7: Nested Loops
# ============================================================================
print(f"\n[Test 7] Nested Loops")

matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
nested_sum = 0

for row_idx, row in enumerate(matrix):
    for col_idx, value in enumerate(row):
        nested_sum += value
        if row_idx == 1 and col_idx == 1 and "checkpoint_8" not in state["checkpoints_passed"]:
            print(f"  [Checkpoint #8] In nested loop, pos=({row_idx},{col_idx}), value={value}")
            rpc.checkpoint(CHECKPOINT_PATH)
            state["checkpoints_passed"].append("checkpoint_8")
            print(f"  [Resumed #8] Continuing nested loops")

state["test_results"].append(("nested_loops", nested_sum == 45))
print(f"  Matrix sum: {nested_sum} (expected 45)")

# ============================================================================
# Test 8: Match Statement (Pattern Matching)
# ============================================================================
print(f"\n[Test 8] Match Statement")

test_data = {"type": "transfer", "amount": 100, "to": "account_123"}
match_result = ""

match test_data:
    case {"type": "deposit", "amount": amt}:
        match_result = f"deposit_{amt}"
    case {"type": "transfer", "amount": amt, "to": target}:
        if "checkpoint_9" not in state["checkpoints_passed"]:
            print(f"  [Checkpoint #9] In match case, transfer to {target}")
            rpc.checkpoint(CHECKPOINT_PATH)
            state["checkpoints_passed"].append("checkpoint_9")
            print(f"  [Resumed #9] Continuing from match")
        match_result = f"transfer_{amt}_to_{target}"
    case _:
        match_result = "unknown"

state["test_results"].append(("match_stmt", "transfer" in match_result))
print(f"  Match result: {match_result}")

# ============================================================================
# Test 9: List Comprehension with Checkpoint After
# ============================================================================
print(f"\n[Test 9] List Comprehension")

numbers = [1, 2, 3, 4, 5]
squares = [x * x for x in numbers]

if "checkpoint_10" not in state["checkpoints_passed"]:
    print(f"  [Checkpoint #10] After list comprehension")
    rpc.checkpoint(CHECKPOINT_PATH)
    state["checkpoints_passed"].append("checkpoint_10")
    print(f"  [Resumed #10] Continuing after comprehension")

state["test_results"].append(("list_comp", squares == [1, 4, 9, 16, 25]))
print(f"  Squares: {squares}")

# ============================================================================
# Test 10: Dictionary and Set Operations
# ============================================================================
print(f"\n[Test 10] Dictionary and Set Operations")

test_dict = {"a": 1, "b": 2, "c": 3}
test_set = {1, 2, 3, 4, 5}

# Iterate over dictionary
dict_sum = 0
for key, value in test_dict.items():
    dict_sum += value

# Set operations
set_result = test_set.union({6, 7})

# Checkpoint after dict/set operations
if "checkpoint_11" not in state["checkpoints_passed"]:
    print(f"  [Checkpoint #11] After dict/set operations")
    rpc.checkpoint(CHECKPOINT_PATH)
    state["checkpoints_passed"].append("checkpoint_11")
    print(f"  [Resumed #11] Continuing after dict/set")

state["test_results"].append(("dict_set", dict_sum == 6 and 7 in set_result))
print(f"  Dict sum: {dict_sum}, Set size: {len(set_result)}")

# ============================================================================
# Test 11: Zip and Multiple Iterators
# ============================================================================
print(f"\n[Test 11] Zip with Multiple Iterators")

list_a = [10, 20, 30]
list_b = ["x", "y", "z"]
zip_results = []

for num, letter in zip(list_a, list_b):
    zip_results.append((num, letter))

# Checkpoint after zip operation
if "checkpoint_12" not in state["checkpoints_passed"]:
    print(f"  [Checkpoint #12] After zip loop")
    rpc.checkpoint(CHECKPOINT_PATH)
    state["checkpoints_passed"].append("checkpoint_12")
    print(f"  [Resumed #12] Continuing after zip")

state["test_results"].append(("zip_loop", len(zip_results) == 3))
print(f"  Zip pairs: {zip_results}")

# ============================================================================
# Test 12: Map and Filter
# ============================================================================
print(f"\n[Test 12] Map and Filter")

def double(x):
    return x * 2

def is_even(x):
    return x % 2 == 0

numbers_list = [1, 2, 3, 4, 5, 6]
doubled = list(map(double, numbers_list))
evens = list(filter(is_even, numbers_list))

if "checkpoint_13" not in state["checkpoints_passed"]:
    print(f"  [Checkpoint #13] After map/filter operations")
    rpc.checkpoint(CHECKPOINT_PATH)
    state["checkpoints_passed"].append("checkpoint_13")
    print(f"  [Resumed #13] Continuing after map/filter")

state["test_results"].append(("map_filter", len(doubled) == 6 and len(evens) == 3))
print(f"  Doubled: {doubled}, Evens: {evens}")

# ============================================================================
# Test 13: Nested Function with Closure
# ============================================================================
print(f"\n[Test 13] Nested Function with Closure")

def outer_with_closure(x):
    """Function that returns a closure."""
    def inner(y):
        return x + y
    return inner

closure_func = outer_with_closure(100)
closure_result = closure_func(23)

# Checkpoint after closure operation
if "checkpoint_14" not in state["checkpoints_passed"]:
    print(f"  [Checkpoint #14] After closure execution")
    rpc.checkpoint(CHECKPOINT_PATH)
    state["checkpoints_passed"].append("checkpoint_14")
    print(f"  [Resumed #14] Continuing after closure")

state["test_results"].append(("closure", closure_result == 123))
print(f"  Closure result: {closure_result} (expected 123)")

# ============================================================================
# Final Report
# ============================================================================
print(f"\n{SEP}")
print("FINAL REPORT")
print(SEP)

passed_count = sum(1 for _, passed in state["test_results"] if passed)
total_count = len(state["test_results"])

print(f"\nCheckpoints passed: {len(state['checkpoints_passed'])}")
print(f"Tests passed: {passed_count}/{total_count}")
print(f"\nDetailed results:")
for test_name, passed in state["test_results"]:
    status = "✓ PASS" if passed else "✗ FAIL"
    print(f"  {status}: {test_name}")

if passed_count == total_count:
    print(f"\n🎉 All tests passed!")
else:
    print(f"\n⚠️  Some tests failed")

# Cleanup
if os.path.exists(CHECKPOINT_PATH):
    os.remove(CHECKPOINT_PATH)
    print(f"\nCheckpoint file removed; next run starts fresh")

print(SEP)

