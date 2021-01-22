
import re

haystack = "Hello world"
needle = 'ello'

mo = re.search(needle, haystack)
print(mo)

# Does not work on python 3.6:
assert isinstance(mo, re.Match)
assert mo.start() == 1
assert mo.end() == 5

assert re.escape('python.exe') == 'python\\.exe'

p = re.compile('ab')
s = p.sub('x', 'abcabca')
# print(s)
assert s == 'xcxca'

idpattern = r'([_a-z][_a-z0-9]*)'

mo = re.search(idpattern, '7382 _boe0+2')
assert mo.group(0) == '_boe0'

# tes op range
assert re.compile('[a-z]').match('a').span() == (0, 1)
assert re.compile('[a-z]').fullmatch('z').span() == (0, 1)

# test op charset
assert re.compile('[_a-z0-9]*').match('_09az').group() == '_09az'

# test op bigcharset
assert re.compile('[你好a-z]*').match('a好z你?').group() == 'a好z你'
assert re.compile('[你好a-z]+').search('1232321 a好z你 !!?').group() == 'a好z你'

# test op repeat one
assert re.compile('a*').match('aaa').span() == (0, 3)
assert re.compile('abcd*').match('abcdddd').group() == 'abcdddd'
assert re.compile('abcd*').match('abc').group() == 'abc'
assert re.compile('abcd*e').match('abce').group() == 'abce'
assert re.compile('abcd*e+').match('abcddeee').group() == 'abcddeee'
assert re.compile('abcd+').match('abcddd').group() == 'abcddd'

# test op mark
assert re.compile('(a)b').match('ab').group(0, 1) == ('ab', 'a')
assert re.compile('a(b)(cd)').match('abcd').group(0, 1, 2) == ('abcd', 'b', 'cd')

# test op repeat
assert re.compile('(ab)+').match('abab')
assert re.compile('(a)(b)(cd)*').match('abcdcdcd').group(0, 1, 2, 3) == ('abcdcdcd', 'a', 'b', 'cd')
assert re.compile('ab()+cd').match('abcd').group() == 'abcd'
assert re.compile('(a)+').match('aaa').groups() == ('a',)
assert re.compile('(a+)').match('aaa').groups() == ('aaa',)

# test Match object method
assert re.compile('(a)(bc)').match('abc')[1] == 'a'
assert re.compile('a(b)(?P<a>c)d').match('abcd').groupdict() == {'a': 'c'}

# test op branch
assert re.compile(r'((?=\d|\.\d)(?P<int>\d*)|a)').match('123.2132').group() == '123'

assert re.sub(r'^\s*', 'X', 'test') == 'Xtest'

assert re.match(r'\babc\b', 'abc').group() == 'abc'

urlpattern = re.compile('//([^/#?]*)(.*)', re.DOTALL)
url = '//www.example.org:80/foo/bar/baz.html'
assert urlpattern.match(url).group(1) == 'www.example.org:80'