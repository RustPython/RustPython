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
#assert f'{"!:"}' == '!:'
#assert f"{1 != 2}" == 'True'
assert fr'x={4*10}\n' == 'x=40\\n'
assert f'{16:0>+#10x}' == '00000+0x10'


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
