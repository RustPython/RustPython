

import types
import unittest
from .core import decorate_test_case_with_reason, mark_test_case_or_reason, get_marked_test_cases

class OriginalMarker:pass
class ModifiedMarker:pass


'''
Front end for RPT for imported test cases.
'''

def get_originals():
    return get_marked_test_cases(OriginalMarker)

def original(test_case):
    mark_test_case_or_reason(test_case, OriginalMarker)
    return test_case


def skip(reason):
    return decorate_test_case_with_reason(reason, unittest.skip)

def fail(reason):
    return decorate_test_case_with_reason(reason, unittest.expectedFailure)

def mark_as_modified(reason):
    return reason

def modified(reason):
    return decorate_test_case_with_reason(reason, mark_as_modified)

def substituted(reason=None,run=False):
    fct=unittest.skip if not run else unittest.expectedFailure
    return decorate_test_case_with_reason(reason, fct)
