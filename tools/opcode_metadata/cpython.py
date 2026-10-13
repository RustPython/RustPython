import os
import pathlib
import sys
from functools import cache

from target_source import verify_import_origins, verify_source

try:
    CPYTHON_ROOT = pathlib.Path(os.environ["CPYTHON_ROOT"]).expanduser().resolve()
except KeyError:
    raise ValueError("Missing environment variable 'CPYTHON_ROOT'")

sys.dont_write_bytecode = True
verify_source(CPYTHON_ROOT)
verify_import_origins(CPYTHON_ROOT)

CPYTHON_TOOLS_LIB = CPYTHON_ROOT / "Tools" / "cases_generator"


if (path := CPYTHON_TOOLS_LIB.as_posix()) not in sys.path:
    sys.path.insert(0, path)


from analyzer import SKIP_PROPERTIES, Analysis, Family, Properties, analyze_files
from stack import get_stack_effect

verify_import_origins(CPYTHON_ROOT)


@cache
def get_analysis() -> Analysis:
    from generators_common import DEFAULT_INPUT

    verify_import_origins(CPYTHON_ROOT)
    analysis = analyze_files([DEFAULT_INPUT])
    # Our speration is done at the enum definition
    analysis.instructions |= analysis.pseudos
    return analysis
