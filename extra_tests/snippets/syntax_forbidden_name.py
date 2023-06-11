from testutils import assert_raises

def raisesSyntaxError(parse_stmt, exec_stmt=None):
    with assert_raises(SyntaxError):
        compile(parse_stmt, '<test>', 'exec')
        if exec_stmt is not None:
            source = "\n".join([parse_stmt, exec_stmt])
            exec(source)

# Check that errors are raised during parsing.
raisesSyntaxError("def f(**__debug__): pass")
raisesSyntaxError("def f(*__debug__): pass")
raisesSyntaxError("def f(__debug__): pass")
raisesSyntaxError("def f(__debug__=1): pass")

# Similarly but during execution.
raisesSyntaxError("def f(**kwargs): pass", "f(__debug__=1)")
raisesSyntaxError("", "__debug__=1")
raisesSyntaxError("", "obj.__debug__ = 1")
raisesSyntaxError("", "__debug__ := 1")
raisesSyntaxError("", "del __debug__")
raisesSyntaxError("", "(a, __debug__, c) = (1, 2, 3)")
raisesSyntaxError("", "(a, *__debug__, c) = (1, 2, 3)")

# TODO:
#  raisesSyntaxError("", "__debug__ : int")
