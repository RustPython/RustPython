import locale
import sys

from testutils import assert_raises

# Windows counts the modifier toward the code-page length before calling the CRT.
if sys.platform == "win32":
    for category, name in (
        (locale.LC_CTYPE, "English.1252@" + "x" * 16),
        (locale.LC_ALL, "LC_CTYPE=English.1252@" + "x" * 16),
    ):
        with assert_raises(locale.Error) as caught:
            locale.setlocale(category, name)
        assert caught.exception.args == ("unsupported locale setting",)
