
def f1():
    """
        x
    \ty
    """
assert f1.__doc__ == '\nx\ny\n'

def f2():
    """
\t    x
\t\ty
    """

assert f2.__doc__ == '\nx\n    y\n'
