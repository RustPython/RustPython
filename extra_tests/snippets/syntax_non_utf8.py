import os
dir_path = os.path.dirname(os.path.realpath(__file__))

try:
    with open(os.path.join(dir_path , "non_utf8.txt")) as f:
        eval(f.read())
except:
    # TODO: RUSTPYTHON, rustpython raise SyntaxError but cpython raise ValueError here.
    pass
else:
  assert False
