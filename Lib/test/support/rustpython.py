"""
RustPython specific helpers.
"""

import doctest


# copied from https://github.com/RustPython/RustPython/pull/6919
EXPECTED_FAILURE = doctest.register_optionflag("EXPECTED_FAILURE")


class DocTestChecker(doctest.OutputChecker):
    """
    Custom output checker that lets us add: `+EXPECTED_FAILURE` for doctest tests.

    We want to be able to mark failing doctest test the same way we do with normal
    unit test, without this class we would have to skip the doctest for the CI to pass.
    """

    def check_output(self, want, got, optionflags):
        res = super().check_output(want, got, optionflags)
        if optionflags & EXPECTED_FAILURE:
            res = not res
        return res
