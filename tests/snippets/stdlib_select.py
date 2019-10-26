from testutils import assert_raises

import select
import sys


class Nope:
    pass

class Almost:
    def fileno(self):
        return 'fileno'

assert_raises(TypeError, select.select, 1, 2, 3)
assert_raises(TypeError, select.select, [Nope()], [], [])
assert_raises(TypeError, select.select, [Almost()], [], [])
assert_raises(TypeError, select.select, [], [], [], "not a number")
assert_raises(ValueError, select.select, [], [], [], -1)

if 'win' in sys.platform:
    assert_raises(OSError, select.select, [0], [], [])

# TODO: actually test select functionality

