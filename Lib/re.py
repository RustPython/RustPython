
""" Regular expressions """


def match(pattern, string, flags=0):
    return _compile(pattern, flags).match(string)


def _compile(pattern, flags):
    p = sre_compile.compile(pattern, flags)
    return p
