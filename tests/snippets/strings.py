assert "a" == 'a'
assert """a""" == "a"
assert len(""" " "" " "" """) == 11
assert "\"" == '"'
assert "\"" == """\""""

assert "\n" == """
"""

assert len(""" " \" """) == 5

assert type("") is str

assert str(1) == "1"
assert str(2.1) == "2.1"
assert str() == ""
assert str("abc") == "abc"

assert repr("a") == "'a'"
assert repr("can't") == '"can\'t"'
assert repr('"won\'t"') == "'\"won\\'t\"'"
assert repr('\n\t') == "'\\n\\t'"

assert str(["a", "b", "can't"]) == "['a', 'b', \"can't\"]"

a = 'Hallo'
assert a.lower() == 'hallo'
assert a.upper() == 'HALLO'
assert a.split('al') == ['H', 'lo']
assert a.startswith('H')
assert not a.startswith('f')
assert a.endswith('llo')
assert not a.endswith('on')

b = '  hallo  '
assert b.strip() == 'hallo'
assert b.lstrip() == 'hallo  '
assert b.rstrip() == '  hallo'

c = 'hallo'
assert c.capitalize() == 'Hallo'
