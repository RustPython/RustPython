code = 'def __new__(_cls, hits, misses, maxsize, currsize): return _tuple_new(_cls, (hits, misses, maxsize, currsize))'
n = {'_tuple_new': tuple.__new__, '__name__': 'namedtuple__CacheInfo'}
exec(code, n)
assert '__new__' in n

def namedtuple(typename, field_names, *, rename=False, defaults=None, module=None):
    n = {'_tuple_new': tuple.__new__, '__name__': 'namedtuple_X'}
    exec(f'{code}', n)
    assert '__new__' in n
X = namedtuple('_CacheInfo', 'hits misses maxsize currsize')

code = '''
def __new__(_cls, hits, misses, maxsize, currsize):
    return _tuple_new(_cls, (hits, misses, maxsize, currsize))
assert '__new__' in locals()
assert '__new__' in globals()
# assert False
'''
n = {'_tuple_new': tuple.__new__, '__name__': 'namedtuple_X'}
exec(code, n)
assert '__new__' in n

def namedtuple(typename, field_names, *, rename=False, defaults=None, module=None):
    n = {'_tuple_new': tuple.__new__, '__name__': 'namedtuple_X'}
    exec(f'{code}', n)
    assert '__new__' in n
X = namedtuple('_CacheInfo', 'hits misses maxsize currsize')

from collections import namedtuple
X = namedtuple('_CacheInfo', 'hits misses maxsize currsize')

x = X(0, 0, 0, 0)
assert x.hits == 0
assert x.misses == 0
assert x.maxsize == 0
assert x.currsize == 0
