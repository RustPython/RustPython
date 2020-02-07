from testutils import assert_raises

src = """
def valid_func():
    pass

yield 2
"""

with assert_raises(SyntaxError) as ae:
    compile(src, 'test.py', 'exec')
assert ae.exception.lineno == 5

src = """
if True:
pass
"""

with assert_raises(IndentationError):
    compile(src, '', 'exec')

src = """
if True:
  pass
    pass
"""

with assert_raises(IndentationError):
    compile(src, '', 'exec')

src = """
if True:
    pass
  pass
"""

with assert_raises(IndentationError):
    compile(src, '', 'exec')

src = """
if True:
    pass
\tpass
"""

with assert_raises(TabError):
    compile(src, '', 'exec')

with assert_raises(SyntaxError):
    compile('0xX', 'test.py', 'exec')


src = """
"aaaa" \a
"bbbb"
"""

with assert_raises(SyntaxError):
    compile(src, 'test.py', 'exec')

src = """
from __future__ import not_a_real_future_feature
"""

with assert_raises(SyntaxError):
    compile(src, 'test.py', 'exec')

src = """
a = 1
from __future__ import print_function
"""

with assert_raises(SyntaxError):
    compile(src, 'test.py', 'exec')

src = """
from __future__ import print_function
"""
compile(src, 'test.py', 'exec')
