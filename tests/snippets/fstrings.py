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
