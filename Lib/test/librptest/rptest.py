

from .imported import original
from .ours import *
from .core import *

import unittest
import types
import re

# regex to parse urls
url_regex = re.compile(
        r'^(?:http|ftp)s?://' # http:// or https://
        r'(?:(?:[A-Z0-9](?:[A-Z0-9-]{0,61}[A-Z0-9])?\.)+(?:[A-Z]{2,6}\.?|[A-Z0-9-]{2,}\.?)|' #domain...
        r'localhost|' #localhost...
        r'\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})' # ...or ip
        r'(?::\d+)?' # optional port
        r'(?:/?|[/?]\S+)$', re.IGNORECASE)


def originated_from(major_version, minor_version, original_file_link):
    '''
    Annotates a test case class with the python version it relates to and the link to the original file.
    '''
    def decorator(test):
        return test

    assert re.match(url_regex, original_file_link)!=None

    return decorator



def print_eval():
    def print_results(res, headline):
        print(headline)
        for r in res:
            print('   '+r)
        print('\n')
    
    print('*******************')
    print('RPT\n')

    news=get_marked_test_cases(MarkNew)
    news=[f.__name__ for f in news]
    print_results(news, 'New Test Cases added to test set')

    subs=get_all_substitutions()
    res=[r[0] + 'is substituted by ' + str([ f[0].__name__ for f in r[1]]) for r in subs.items()]
    print_results(res, 'Substituted Test Cases')

