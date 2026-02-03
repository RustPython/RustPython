"""
RustPython specific helpers.
"""

import doctest


# copied from https://github.com/RustPython/RustPython/pull/6919
EXPECTED_FAILURE = doctest.register_optionflag("EXPECTED_FAILURE")


class DocTestChecker(doctest.OutputChecker):
    """
    Custom output checker that lets us to add: `+EXPECTED_FAILURE` for doctest tests.
    """

    def check_output(self, want, got, optionflags):
        if optionflags & EXPECTED_FAILURE:
            return want != got
        return super().check_output(want, got, optionflags)
