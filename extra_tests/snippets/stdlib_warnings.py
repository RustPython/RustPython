import _warnings

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
    expected = [("get", "__loader__"), ("get", "__spec__")]
    if has_name:
        expected.append(("source", "stored_name"))
        del module_globals["__name__"]
    assert events == expected
