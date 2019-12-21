from testutils import assert_raises
foo = 'bar'

assert f"{''}" == ''
assert f"{f'{foo}'}" == 'bar'
assert f"foo{foo}" == 'foobar'
assert f"{foo}foo" == 'barfoo'
assert f"foo{foo}foo" == 'foobarfoo'
assert f"{{foo}}" == '{foo}'
assert f"{ {foo} }" == "{'bar'}"
assert f"{f'{{}}'}" == '{}' # don't include escaped braces in nested f-strings
assert f'{f"{{"}' == '{'
assert f'{f"}}"}' == '}'
assert f'{foo}' f"{foo}" 'foo' == 'barbarfoo'
assert f'{"!:"}' == '!:'
assert fr'x={4*10}\n' == 'x=40\\n'
assert f'{16:0>+#10x}' == '00000+0x10'
assert f"{{{(lambda x: f'hello, {x}')('world}')}" == '{hello, world}'

spec = "0>+#10x"
assert f"{16:{spec}}{foo}" == '00000+0x10bar'

# TODO:
# spec = "bla"
# assert_raises(ValueError, lambda: f"{16:{spec}}")

# Normally `!` cannot appear outside of delimiters in the expression but
# cpython makes an exception for `!=`, so we should too.

# assert f'{1 != 2}' == 'True'


# conversion flags

class Value:
    def __format__(self, spec):
        return "foo"

    def __repr__(self):
        return "bar"

    def __str__(self):
        return "baz"

v = Value()

assert f'{v}' == 'foo'
assert f'{v!r}' == 'bar'
assert f'{v!s}' == 'baz'
assert f'{v!a}' == 'bar'

# advanced expressions:

assert f'{True or True}' == 'True'
assert f'{1 == 1}' == 'True'
assert f'{"0" if True else "1"}' == '0'

# Test ascii representation of unicodes:
v = "\u262e"
assert f'>{v}' == '>\u262e'
assert f'>{v!r}' == ">'\u262e'"
assert f'>{v!s}' == '>\u262e'
assert f'>{v!a}' == r">'\u262e'"
