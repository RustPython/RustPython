"""Main entry point"""

import sys
if sys.argv[0].endswith("__main__.py"):
    # FIXME change to `import os.path` as it was for cpython once `import os.path` works
    import os
    # We change sys.argv[0] to make help message more useful
    # use executable without path, unquoted
    # (it's just a hint anyway)
    # (if you have spaces in your executable you get what you deserve!)
    executable = os.path.basename(sys.executable)
    sys.argv[0] = executable + " -m unittest"
    del os

__unittest = True

from .main import main

main(module=None)
