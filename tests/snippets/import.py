import import_target, import_target as aliased
from import_target import func, other_func
from import_target import func as aliased_func, other_func as aliased_other_func

assert import_target.X == import_target.func()
assert import_target.X == func()

assert import_target.Y == other_func()

assert import_target.X == aliased.X
assert import_target.Y == aliased.Y

assert import_target.X == aliased_func()
assert import_target.Y == aliased_other_func()

#try:
#    X
#except NameError:
#    pass
#else:
#    raise AssertionError('X should not be imported')
