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
