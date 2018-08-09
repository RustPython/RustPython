import import_target
from import_target import func

assert import_target.X == import_target.func()
assert import_target.X == func()

#try:
#    X
#except NameError:
#    pass
#else:
#    raise AssertionError('X should not be imported')
