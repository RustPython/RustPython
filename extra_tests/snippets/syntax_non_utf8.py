import os
import platform

from testutils import assert_raises

dir_path = os.path.dirname(os.path.realpath(__file__))

# TODO: RUSTPYTHON. At some point snippets will fail and it will look confusing
# and out of the blue. This is going to be the cause and it's going to happen when
# the github worker for MacOS starts using Python 3.11.4.
if platform.python_implementation() == "CPython" and platform.system() == 'Darwin':
    expectedException = ValueError
else:
    expectedException = SyntaxError

with assert_raises(expectedException):
    with open(os.path.join(dir_path , "non_utf8.txt")) as f:
        eval(f.read())
