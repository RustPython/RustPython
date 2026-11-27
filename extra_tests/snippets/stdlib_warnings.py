import _warnings
import sys

_warnings.warn("Test")


import warnings


class CustomWarning(UserWarning):
    pass


for category in (None, DeprecationWarning, int, 42):
    message = CustomWarning("sentinel")
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        warnings.warn(message, category)
    assert len(caught) == 1
    assert caught[0].message is message
    assert caught[0].category is CustomWarning


from importlib.machinery import ModuleSpec

events = []


class WarningLoader:
    def get_source(self, name):
        events.append(("source", name))
        return "source line\n"


class WarningGlobals(dict):
    def get(self, key, default=None):
        events.append(("get", key))
        return super().get(key, default)

    def __getitem__(self, key):
        raise AssertionError("module name must use stored dict lookup")

    __missing__ = __getitem__


loader = WarningLoader()
module_globals = WarningGlobals(
    __name__="stored_name",
    __loader__=loader,
    __spec__=ModuleSpec("stored_name", loader),
)
for has_name in (True, False):
    events.clear()
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        warnings.warn_explicit(
            "subclass globals",
            UserWarning,
            "fixed_ascii.py",
            1,
            module_globals=module_globals,
        )
    assert len(caught) == 1
    expected = [
        ("get", "__loader__"),
        ("get", "__spec__"),
    ]
    # CPython 3.15.0, still used by CI, requires __name__. Since 3.15.1
    # it prefers __spec__.name; RustPython must follow the newer behavior.
    if has_name or not (
        sys.implementation.name == "cpython" and sys.version_info < (3, 15, 1)
    ):
        expected.append(("source", "stored_name"))
    if has_name:
        del module_globals["__name__"]
    assert events == expected


def check_unclosed_warning_finalizer():
    import gc
    import io
    import os
    import tempfile

    calls = []

    def report_unraisable(info):
        calls.append((info.exc_type, info.err_msg, info.object))

    def check(factory, kind, error_type):
        warnings_seen = []

        def show_warning(message):
            assert message.category is ResourceWarning
            assert message.source is not None
            warnings_seen.append(message.category)
            raise error_type("warning display failed")

        original_show = warnings._showwarnmsg
        original_hook = sys.unraisablehook
        calls.clear()
        with warnings.catch_warnings():
            warnings.simplefilter("always", ResourceWarning)
            warnings._showwarnmsg = show_warning
            sys.unraisablehook = report_unraisable
            try:
                stream = factory()
                del stream
                gc.collect()
            finally:
                warnings._showwarnmsg = original_show
                sys.unraisablehook = original_hook

        assert warnings_seen == [ResourceWarning]
        if issubclass(error_type, Warning):
            ((reported, message, obj),) = calls
            assert reported is error_type
            assert message.startswith(f"Exception ignored while finalizing {kind} ")
            assert obj is None
        else:
            assert not calls

    with tempfile.TemporaryDirectory() as folder:
        path = os.path.join(folder, "resource")
        with open(path, "wb"):
            pass
        factories = [
            (lambda: io.FileIO(path), "file"),
            (lambda: open(path, "rb"), "file"),
            (lambda: open(path, encoding="utf-8"), "file"),
            (lambda: os.scandir(folder), "scandir iterator"),
        ]
        for factory, kind in factories:
            for error_type in (ValueError, ResourceWarning, UserWarning):
                check(factory, kind, error_type)

        if os.scandir in os.supports_fd:
            fd = os.open(folder, os.O_RDONLY)
            try:
                for error_type in (ValueError, ResourceWarning, UserWarning):
                    check(lambda: os.scandir(fd), "scandir iterator", error_type)
            finally:
                os.close(fd)


if sys.platform not in ("emscripten", "wasi"):
    check_unclosed_warning_finalizer()
