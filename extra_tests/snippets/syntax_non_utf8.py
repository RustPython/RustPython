import os
dir_path = os.path.dirname(os.path.realpath(__file__))

# TODO: RUSTPYTHON, RustPython raises a SyntaxError here
with assert_raises(ValueError):
    with open(os.path.join(dir_path , "non_utf8.txt")) as f:
        eval(f.read())
