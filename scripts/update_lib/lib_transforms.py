"""Explicit, pinned adaptations for RustPython's standard library layout."""

import hashlib

# CPython d220409a13b77de7a6c30abe16ea90db01528edc, Lib/pydoc.py.
PYDOC_SOURCE_SHA256 = "cf007075ab2a6d7d408be847ded67efa31dd99ae535540e6d80fa510359551a4"
PYDOC_STDLIB_ANCHOR = b"""    STDLIB_DIR = sysconfig.get_path('stdlib')

    def document(self, object, name=None, *args):
"""
PYDOC_STDLIB_REPLACEMENT = b"""    STDLIB_DIR = sysconfig.get_path('stdlib')
    # TODO: RUSTPYTHON; Cargo builds keep Lib outside sysconfig's install paths.
    if (sys.implementation.name == 'rustpython'
            and getattr(sys, '_is_python_build', False)
            and not sysconfig.is_python_build()
            and getattr(sys, '_stdlib_dir', None)):
        STDLIB_DIR = sys._stdlib_dir

    def document(self, object, name=None, *args):
"""


def pydoc_build_stdlib(source: bytes) -> bytes:
    """Adapt only the default doc lookup root in the pinned canonical pydoc.

    An explicit basedir keeps its upstream meaning. CPython and installed
    RustPython layouts retain upstream behavior. Regenerate from the canonical
    input; already adapted or changed inputs require a fresh review.
    """
    if source.count(PYDOC_STDLIB_ANCHOR) != 1:
        raise ValueError("Expected one canonical pydoc STDLIB_DIR anchor")
    if hashlib.sha256(source).hexdigest() != PYDOC_SOURCE_SHA256:
        raise ValueError("pydoc SHA256 does not match the pinned canonical input")
    return source.replace(PYDOC_STDLIB_ANCHOR, PYDOC_STDLIB_REPLACEMENT, 1)
