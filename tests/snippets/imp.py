import _imp

assert _imp.is_builtin("time") == True
assert _imp.is_builtin("os") == False
assert _imp.is_builtin("not existing module") == False
