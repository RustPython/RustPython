from testutils import assert_equal

import dir_module
assert dir_module.value == 5
assert dir_module.value2 == 7

try:
    dir_module.unknown_attr
except AttributeError as e:
    assert 'dir_module' in str(e)
else:
    assert False

del dir_module.__name__
try:
    dir_module.unknown_attr
except AttributeError as e:
    assert 'dir_module' not in str(e)
else:
    assert False

dir_module.__name__ = 1
try:
    dir_module.unknown_attr
except AttributeError as e:
    assert 'dir_module' not in str(e)
else:
    assert False
