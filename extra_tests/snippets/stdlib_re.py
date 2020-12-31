
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
# s = p.sub('x', 'abcabca')
# print(s)
# assert s == 'xcxca'

idpattern = r'([_a-z][_a-z0-9]*)'

# mo = re.search(idpattern, '7382 _boe0+2')
# print(mo)
# TODO:
# assert mo.group(0) == '_boe0'

assert re.compile('[a-z]').match('a').span() == (0, 1)
assert re.compile('[a-z]').fullmatch('z').span() == (0, 1)

# test op repeat one
assert re.compile('a*').match('aaa').span() == (0, 3)
assert re.compile('abcd*').match('abcdddd').group() == 'abcdddd'
assert re.compile('abcd*').match('abc').group() == 'abc'
assert re.compile('abcd*e').match('abce').group() == 'abce'
assert re.compile('abcd+').match('abcddd').group() == 'abcddd'
