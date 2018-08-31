def no_args():
    pass

no_args()

try:
    no_args('one_arg')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: 1 arg to no_args'

try:
    no_args(kw='should fail')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: kwarg to no_args'


def one_arg(arg):
    return arg

one_arg('one_arg')
assert "arg" == one_arg(arg="arg")

try:
    one_arg()
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: no args to one_arg'

try:
    one_arg(wrong_arg='wont work')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: incorrect kwarg to one_arg'

try:
    one_arg('one_arg', 'two_arg')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: two args to one_arg'

try:
    one_arg('one_arg', extra_arg='wont work')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: extra kwarg to one_arg'

try:
    one_arg('one_arg', arg='duplicate')
except TypeError:
    pass
else:
    assert False, 'no TypeError raised: same pos and kwarg to one_arg'


def one_default_arg(arg="default"):
    return arg

assert 'default' == one_default_arg()
assert 'arg' == one_default_arg('arg')
assert 'kwarg' == one_default_arg(arg='kwarg')

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


def two_pos(a, b):
    return (a, b)

assert ('a', 'b') == two_pos('a', 'b')
assert ('a', 'b') == two_pos(b='b', a='a')


def kwargs_are_variable(x=[]):
    x.append(1)
    return x

assert [1] == kwargs_are_variable()
assert [1, 1] == kwargs_are_variable()
