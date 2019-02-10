import os 

assert os.open('README.md', 0) > 0


try:
    os.open('DOES_NOT_EXIST', 0)
    assert False
except FileNotFoundError:
    pass
