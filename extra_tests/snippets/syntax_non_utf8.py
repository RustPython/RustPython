import os
import platform

from testutils import assert_raises

dir_path = os.path.dirname(os.path.realpath(__file__))

# TODO: RUSTPYTHON, RustPython raises a SyntaxError here, but cpython raise a ValueError
error = SyntaxError if platform.python_implementation() == 'RustPython' else ValueError
with assert_raises(error):
    with open(os.path.join(dir_path , "non_utf8.txt")) as f:
        eval(f.read())
