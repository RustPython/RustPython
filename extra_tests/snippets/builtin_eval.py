assert 3 == eval("1+2")

code = compile("5+3", "x.py", "eval")
assert eval(code) == 8

# Test that globals must be a dict
import collections

# UserDict is a mapping but not a dict - should fail in eval
user_dict = collections.UserDict({"x": 5})
try:
    eval("x", user_dict)
    assert False, "eval with UserDict globals should fail"
except TypeError as e:
    # CPython: "globals must be a real dict; try eval(expr, {}, mapping)"
    assert "globals must be a real dict" in str(e), e

# Non-mapping should have different error message
try:
    eval("x", 123)
    assert False, "eval with int globals should fail"
except TypeError as e:
    # CPython: "globals must be a dict"
    assert "globals must be a dict" in str(e)
    assert "real dict" not in str(e)

# List is not a mapping
try:
    eval("x", [])
    assert False, "eval with list globals should fail"
except TypeError as e:
    assert "globals must be a real dict" in str(e), e

# Regular dict should work
assert eval("x", {"x": 42}) == 42

# None should use current globals
x = 100
assert eval("x", None) == 100

# Test locals parameter
# Locals can be any mapping (unlike globals which must be dict)
assert eval("y", {"y": 1}, user_dict) == 1  # UserDict as locals is OK

# But locals must still be a mapping
try:
    eval("x", {"x": 1}, 123)
    assert False, "eval with int locals should fail"
except TypeError as e:
    # This error is handled by ArgMapping validation
    assert "not a mapping" in str(e) or "locals must be a mapping" in str(e)

# Test that __builtins__ is added if missing
globals_without_builtins = {"x": 5}
result = eval("x", globals_without_builtins)
assert result == 5
assert "__builtins__" in globals_without_builtins

# Test with both globals and locals
assert eval("x + y", {"x": 10}, {"y": 20}) == 30

# Test that when globals is None and locals is provided, it still works
assert eval("x + y", None, {"x": 1, "y": 2}) == 3


# Test code object with free variables
def make_closure():
    z = 10
    return compile("x + z", "<string>", "eval")


closure_code = make_closure()
try:
    eval(closure_code, {"x": 5})
    assert False, "eval with code containing free variables should fail"
except NameError as e:
    pass
