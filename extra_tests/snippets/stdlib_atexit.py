"""atexit.unregister() compares callbacks with arbitrary Python code.

The comparison runs with the callback list unlocked, so __eq__ may clear it
and register something new. unregister() then has to tell whether the entry
it compared is still there, and must not mistake a later registration that
happens to occupy the same storage for that entry.
"""

import atexit


def make(name):
    def f():
        ran.append(name)

    f.tag = name
    return f


ran = []
a, b, c, d = (make(n) for n in "abcd")


class Probe:
    def __init__(self, action=None, result=True):
        self.action = action
        self.result = result
        self.seen = []

    def __eq__(self, other):
        self.seen.append(getattr(other, "tag", "?"))
        if self.action is not None:
            self.action()
        return self.result


def remaining():
    del ran[:]
    atexit._run_exitfuncs()
    return list(ran)


# A callback the probe does not match is left alone.
atexit._clear()
atexit.register(a)
atexit.register(b)
probe = Probe(result=False)
atexit.unregister(probe)
assert probe.seen == ["a", "b"], probe.seen
assert remaining() == ["b", "a"], ran

# Matching callbacks are dropped, oldest compared first.
atexit._clear()
atexit.register(a)
atexit.register(b)
atexit.register(c)
probe = Probe(result=True)
atexit.unregister(probe)
assert probe.seen == ["a", "b", "c"], probe.seen
assert atexit._ncallbacks() == 0
assert remaining() == [], ran

# __eq__ empties the list: there is nothing left to drop.
atexit._clear()
atexit.register(a)
atexit.register(b)
probe = Probe(action=atexit._clear, result=True)
atexit.unregister(probe)
assert probe.seen == ["a"], probe.seen
assert remaining() == [], ran

# __eq__ empties the list and registers a replacement. The replacement is a
# different callback, so it survives however its storage was reused.
atexit._clear()
atexit.register(a)
atexit.register(b)
atexit.register(c)


def replace():
    atexit._clear()
    atexit.register(d)


probe = Probe(action=replace, result=True)
atexit.unregister(probe)
assert probe.seen == ["a", "d"], probe.seen
assert remaining() == ["d"], ran

# __eq__ registers without clearing: every entry the walk had already passed
# stays, and so does each newly registered one.
atexit._clear()
atexit.register(a)
atexit.register(b)
probe = Probe(action=lambda: atexit.register(c), result=True)
atexit.unregister(probe)
assert probe.seen == ["a", "c"], probe.seen
assert remaining() == ["c", "c", "b", "a"], ran

atexit._clear()
print("ok")
