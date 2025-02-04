
def f1():
    """
        x
    \ty
    """
print(repr(f1.__doc__))

def f2():
    """
\t    x
\t\ty
    """

print(repr(f2.__doc__))
