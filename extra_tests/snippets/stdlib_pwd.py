import sys

# windows doesn't support pwd
if sys.platform.startswith("win"):
    exit(0)

from testutils import assert_raises
import pwd

with assert_raises(KeyError):
    fake_name = "fake_user"
    while pwd.getpwnam(fake_name):
        fake_name += "1"
