"""
https://github.com/RustPython/RustPython/issues/4505
"""

def foo():
    def inner():
        pass

assert foo.__code__.co_names == ()

stmts = """
import blah
 
def foo():
    pass
"""

code = compile(stmts, "<test>", "exec")
assert code.co_names == ("blah", "foo")
