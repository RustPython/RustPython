import sys

# windows doesn't support pwd
if sys.platform.startswith("win"):
    exit(0)

import pwd

from testutils import assert_raises

with assert_raises(KeyError):
    fake_name = "fake_user"
    while pwd.getpwnam(fake_name):
        fake_name += "1"

# The field getters must not index a struct sequence that __new__ never filled.
with assert_raises(TypeError):
    pwd.struct_passwd()
