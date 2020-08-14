'''
Core Functions Logic to manage and evaluate test case annotations.
'''

import types
import collections



substitutions = {}
marked_test_cases = {}

### Adds a function as a substitute of the original function fct.
def add_subst(fct, subst, reason=None):
    if fct==None:
        return
    
    if fct in substitutions:
        substitutions[fct].add((subst, reason))
    else:
        substitutions[fct]=set([(subst, reason)])

### Returns True if a function is substituted otherwise False.
def is_substituted(fct):
    return fct in substitutions

### Returns a list of functions that are being used as substitutes of the 
### original function fct. Does not fail and returns an empty set if the function
### has no known substitutes.
def get_substitutions(fct):
    try:
        return substitutions[fct]
    except:
        return set()

### Add a function that is marked as being substituted 
### When the function is already known, nothing happens.
def register_subst(fct):
    if not fct in substitutions:
        substitutions[fct]=set()

def get_all_substitutions():
    return substitutions

def decorate_test_case_with_reason(tc_or_reason, deco, *args, **kwargs):
    if isinstance(tc_or_reason, types.FunctionType):
        return deco(tc_or_reason, *args, **kwargs)
    else:
        # expect that it is a reason
        def decorator(test_case):
            return deco(test_case)
        return decorator

def mark_test_case_or_reason(tc_or_reason, marker):
    def mark(test_case):
        if marker in marked_test_cases:
            marked_test_cases[marker].add(test_case)
        else:
            marked_test_cases[marker] = set([test_case])

    def decorator(test_case):
        mark(test_case)
        return test_case

    if isinstance(tc_or_reason, types.FunctionType):
        mark(tc_or_reason)
        return tc_or_reason
    else:
        return decorator

    

def get_marked_test_cases(marker):
    try:
        return marked_test_cases[marker]
    except:
        return set()
