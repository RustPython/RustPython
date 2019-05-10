import os

from import_file import import_file

import_file()

assert os.path.basename(__file__) == "builtin_file.py"
