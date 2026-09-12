"""Stub psutil.

pyperf hard-depends on psutil (a C-extension package RustPython cannot build
or load, since RustPython has no CPython C-API/extension-module support).

pyperf itself already disables psutil usage on interpreters that report
Py_GIL_DISABLED=1 in sysconfig (see pyperf._utils.USE_PSUTIL), which is the
value RustPython reports since it has no GIL. So on RustPython, pyperf never
actually calls into psutil at runtime -- this stub only needs to satisfy
pip's dependency resolution, not provide working functionality.

Do not use this against a real Python/pyperf run where USE_PSUTIL is True:
every function here raises immediately.
"""


class AccessDenied(Exception):
    pass


class NoSuchProcess(Exception):
    pass


REALTIME_PRIORITY_CLASS = None


def _unsupported(*_a, **_k):
    raise NotImplementedError(
        "stub psutil: no real implementation; only present to satisfy "
        "pip dependency resolution on interpreters without C-extension support"
    )


cpu_count = _unsupported
boot_time = _unsupported


class Process:
    def __init__(self, *_a, **_k):
        _unsupported()
