import os
import pathlib
import sys

try:
    CPYTHON_ROOT = pathlib.Path(os.environ["CPYTHON_ROOT"]).expanduser().resolve()
except KeyError:
    raise ValueError("Missing environment variable 'CPYTHON_ROOT'")

CPYTHON_TOOLS_LIB = CPYTHON_ROOT / "Tools" / "cases_generator"


if (path := CPYTHON_TOOLS_LIB.as_posix()) not in sys.path:
    sys.path.append(path)


from analyzer import SKIP_PROPERTIES, Analysis, Family, Properties, analyze_files
from stack import get_stack_effect


def get_analysis() -> Analysis:
    from generators_common import DEFAULT_INPUT

    analysis = analyze_files([DEFAULT_INPUT])
    # Our speration is done at the enum definition
    analysis.instructions |= analysis.pseudos
    return analysis
