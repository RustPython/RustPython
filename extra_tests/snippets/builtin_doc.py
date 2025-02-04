
def f1():
    """
        x
    \ty
    """
repr(f1.__doc__) == '\nx \ny\n'

def f2():
    """
\t    x
\t\ty
    """

repr(f2.__doc__) == '\nx \n y\n'
