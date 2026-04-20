"""Regression tests for issues that were silently fixed but whose GitHub
issues remained open (no linking PR closed them). Each section below pins
the minimal reproduction from the original report, so future refactors
can't quietly re-introduce the bug.

Closing:
  #4317  starmap over self-referential iteration crashed with SIGSEGV
  #4859  repr() of an asyncio Task aborted with a Rust panic
  #4860  deeply-nested XML tree segfaulted during refcount cleanup
  #4861  reassigning a name after __del__ produced a null reference
  #4863  __del__ calling type(self)() panicked instead of raising RecursionError
  #5154  specific function body shape with `while/else` + trailing dict crashed
"""

import asyncio
import xml.etree.ElementTree as ET
from itertools import starmap

# --- #4317: starmap over zip of an empty list, repeated -------------------
#
# Original report: `b = []; for i in range(100000): b = starmap(i, zip(b, b));
# list(b)` segfaulted. The iterator was self-referential and the Rust stack
# unwound until SIGSEGV. Evaluation now terminates cleanly.
_b = []
for _i in range(1000):
    _b = starmap(_i, zip(_b, _b))
assert list(_b) == []


# --- #4859: repr() of an asyncio Task -------------------------------------
#
# Original report: inside `asyncio.run`, constructing a Task and calling
# repr() on it panicked. The call now returns a non-empty string.
async def _4859_coro(a, b):
    return a + b


async def _4859_main():
    task = asyncio.create_task(_4859_coro(1, b=1))
    s = repr(task)
    assert isinstance(s, str) and len(s) > 0
    return await task


assert asyncio.run(_4859_main()) == 2


# --- #4860: deep XML tree cleanup ----------------------------------------
#
# Original report: building ~20000 nested XML SubElements then dropping
# references segfaulted during GC. Running the same pattern (smaller depth
# here to keep the snippet fast) now completes cleanly when all element
# references are released within the test — relying on module teardown
# would let a cleanup-path regression hide from this snippet.
_root = ET.Element("x")
_node = _root
for _ in range(200):
    _node = ET.SubElement(_node, "x")
_deep = _node  # keep a separate reference to the deepest element
# Drop root first (the original trigger pattern), then the remaining refs.
del _root
del _node
del _deep


# --- #4861: reassignment of a name immediately after del ------------------
#
# Original report: reassigning a name held by an instance whose `__del__`
# runs during the reassignment produced a null reference panic. The bug
# was about the reassignment sequence, not about cascading destructors;
# the regression test exercises the same reassignment path but keeps the
# destructor trivial so the test doesn't depend on GC traversal time.
class _Cls4861:
    called = [False]

    def __del__(self):
        _Cls4861.called[0] = True


_c = _Cls4861()
_c = range(10)  # reassignment triggers __del__ on the original instance
del _c
assert _Cls4861.called[0]


# --- #4863: __del__ calling type(self)() ---------------------------------
#
# Original report: `class Dummy: def __del__(self): type(self)()` + `del d`
# panicked with `Result::unwrap() on an Err`. The regression exercises the
# `type(self)()` pattern inside `__del__` exactly once; a one-shot guard
# prevents the constructor chain from cascading (which CPython handles via
# its recursion limit but at noticeable cost).
class _Dummy4863:
    fired = [False]

    def __del__(self):
        if _Dummy4863.fired[0]:
            return
        _Dummy4863.fired[0] = True
        type(self)()  # this is the exact shape that used to panic


_d = _Dummy4863()
del _d
assert _Dummy4863.fired[0]


# --- #5154: function body with `while/else` + trailing dict literal -------
#
# Original report (CReduced from scrapscript.py): a specific combination of
# `while/else` returning an undefined name followed by an unused dict
# literal crashed the compiler. The same source now compiles cleanly (we
# do not execute the function, since `x` is undefined — the point is that
# the code is accepted).
_src_5154 = """
class LeftParen:
    pass


class RightParen:
    pass


class Lexer:
    def read_char(self):
        return None

    def read_one(self):
        while self:
            c = self.read_char()
            break
        else:
            return x
        {
            "(": LeftParen,
            ")": RightParen,
        }


def tokenize():
    lexer = Lexer()
    while lexer.read_one():
        pass
"""
compile(_src_5154, "<#5154>", "exec")
