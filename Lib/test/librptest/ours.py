
import types

from .core import mark_test_case_or_reason, get_marked_test_cases, add_subst, get_substitutions as core_get_subs


'''
Front End for own test cases in imported test files.
'''


class MarkNew:pass
class MarkExtension:pass



def new(test_case):
    return mark_test_case_or_reason(test_case, MarkNew)

def get_news():
    return get_marked_test_cases(MarkNew)

def subst(original_tc, reason=None):
    def sub_decorator(new_tc):
        add_subst(original_tc, new_tc, reason)
    return sub_decorator

def get_substitutions(fct):
    if isinstance(fct, types.FunctionType):
        fct=fct.__name__

    return core_get_subs(fct)