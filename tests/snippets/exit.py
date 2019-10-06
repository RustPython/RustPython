from testutils import assert_raises

with assert_raises(SystemExit):
    exit()

with assert_raises(SystemExit):
    exit(None)

with assert_raises(SystemExit):
    exit(1)

with assert_raises(NameError):
    exit(AB)

with assert_raises(SystemExit):
    exit("AB")

with assert_raises(SystemExit):
    quit()

with assert_raises(SystemExit):
    quit(None)

with assert_raises(SystemExit):
    quit(1)

with assert_raises(NameError):
    quit(AB)

with assert_raises(SystemExit):
    quit("AB")

import sys

with assert_raises(SystemExit):
    sys.exit()

with assert_raises(SystemExit):
    sys.exit(None)

with assert_raises(SystemExit):
    sys.exit(1)

with assert_raises(NameError):
    sys.exit(AB)

with assert_raises(SystemExit):
    sys.exit("AB")