import import_target, import_target as aliased
from import_target import func, other_func
from import_target import func as aliased_func, other_func as aliased_other_func
from import_star import *

assert import_target.X == import_target.func()
assert import_target.X == func()

assert import_target.Y == other_func()

assert import_target.X == aliased.X
assert import_target.Y == aliased.Y

assert import_target.X == aliased_func()
assert import_target.Y == aliased_other_func()

assert STAR_IMPORT == '123'

try:
    from import_target import func, unknown_name
    raise AssertionError('`unknown_name` does not cause an exception')
except ImportError:
    pass

# TODO: Once we can determine current directory, use that to construct this
# path:
#import sys
#sys.path.append("snippets/import_directory")
#import nested_target

#try:
#    X
#except NameError:
#    pass
#else:
#    raise AssertionError('X should not be imported')
