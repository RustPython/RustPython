assert "a" == 'a'
assert """a""" == "a"
assert len(""" " "" " "" """) == 11
assert "\"" == '"'
assert "\"" == """\""""

assert "\n" == """
"""

assert len(""" " \" """) == 5
assert len("é") == 1
assert len("é") == 2
assert len("あ") == 1

assert type("") is str
assert type(b"") is bytes

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
assert a.zfill(8) == '000Hallo'
assert a.isalnum()
assert not a.isdigit()
assert not a.isdecimal()
assert not a.isnumeric()
assert a.istitle()
assert a.isalpha()



b = '  hallo  '
assert b.strip() == 'hallo'
assert b.lstrip() == 'hallo  '
assert b.rstrip() == '  hallo'

c = 'hallo'
assert c.capitalize() == 'Hallo'
assert c.center(11, '-') == '---hallo---'
# assert c.isascii()
assert c.index('a') == 1
assert c.rindex('l') == 3
assert c.find('h') == 0
assert c.rfind('x') == -1
assert c.islower()
assert c.title() == 'Hallo'
assert c.count('l') == 2

assert '   '.isspace()
assert 'hello\nhallo\nHallo'.splitlines() == ['hello', 'hallo', 'Hallo']
assert 'abc\t12345\txyz'.expandtabs() == 'abc     12345   xyz'
assert '-'.join(['1', '2', '3']) == '1-2-3'
assert 'HALLO'.isupper()
assert "hello, my name is".partition("my ") == ('hello, ', 'my ', 'name is')
assert "hello, my name is".rpartition("is") == ('hello, my name ', 'is', '')
assert not ''.isdecimal()
assert '123'.isdecimal()
assert not '\u00B2'.isdecimal()

# String Formatting
assert "{} {}".format(1,2) == "1 2"
assert "{0} {1}".format(2,3) == "2 3"
assert "--{:s>4}--".format(1) == "--sss1--"
assert "{keyword} {0}".format(1, keyword=2) == "2 1"

assert 'a' < 'b'
assert 'a' <= 'b'
assert 'a' <= 'a'
assert 'z' > 'b'
assert 'z' >= 'b'
assert 'a' >= 'a'
