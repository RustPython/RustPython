import import_target, import_target as aliased
from import_target import func, other_func
from import_target import func as aliased_func, other_func as aliased_other_func
from import_star import *

import import_mutual1
assert import_target.X == import_target.func()
assert import_target.X == func()

assert import_mutual1.__name__ == "import_mutual1"

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

try:
    import mymodule
except ModuleNotFoundError as exc:
    assert exc.name == 'mymodule'


test = __import__("import_target")
assert test.X == import_target.X

import builtins
class OverrideImportContext():

	def __enter__(self):
		self.original_import = builtins.__import__

	def __exit__(self, exc_type, exc_val, exc_tb):
		builtins.__import__ = self.original_import

with OverrideImportContext():
	def fake_import(name, globals=None, locals=None, fromlist=(), level=0):
		return len(name)

	builtins.__import__ = fake_import
	import test
	assert test == 4


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

from testutils import assert_raises

with assert_raises(SyntaxError):
	exec('import')

