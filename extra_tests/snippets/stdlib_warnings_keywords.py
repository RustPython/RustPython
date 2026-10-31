import warnings

from testutils import assert_raises

names = ("message", "category", "filename", "lineno")
for message, category in (("probe", UserWarning), (UserWarning("probe"), None)):
    values = (message, category, "probe.py", 7)
    for positional_count in range(5):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn_explicit(
                *values[:positional_count],
                **dict(zip(names[positional_count:], values[positional_count:])),
            )
        assert len(caught) == 1
        warning = caught[0]
        assert str(warning.message) == "probe"
        assert warning.category is UserWarning
        assert warning.filename == "probe.py"
        assert warning.lineno == 7

arguments = dict(zip(names, ("probe", UserWarning, "probe.py", 7)))
for name in names:
    missing = arguments.copy()
    del missing[name]
    with assert_raises(TypeError):
        warnings.warn_explicit(**missing)

with assert_raises(TypeError):
    warnings.warn_explicit("probe", **arguments)
