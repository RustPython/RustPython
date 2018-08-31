def no_args():
    pass

no_args()

try:
    no_args('one_arg')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: 1 arg to no_args'


def one_arg(arg):
    pass

one_arg('one_arg')

try:
    one_arg()
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: no args to one_arg'

try:
    one_arg('one_arg', 'two_arg')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: two args to one_arg'


def one_default_arg(arg="default"):
    return arg

assert 'default' == one_default_arg()
assert 'arg' == one_default_arg('arg')

try:
    one_default_arg('one_arg', 'two_arg')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: two args to one_default_arg'


def one_normal_one_default_arg(pos, arg="default"):
    return pos, arg

assert ('arg', 'default') == one_normal_one_default_arg('arg')
assert ('arg', 'arg2') == one_normal_one_default_arg('arg', 'arg2')

try:
    one_normal_one_default_arg()
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: no args to one_normal_one_default_arg'

try:
    one_normal_one_default_arg('one', 'two', 'three')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: three args to one_normal_one_default_arg'
