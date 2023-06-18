import os

from testutils import assert_raises

dir_path = os.path.dirname(os.path.realpath(__file__))

with assert_raises(SyntaxError):
    with open(os.path.join(dir_path , "non_utf8.txt")) as f:
        eval(f.read())
