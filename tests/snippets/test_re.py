
import re

haystack = "Hello world"
needle = 'ello'

mo = re.search(needle, haystack)
print(mo)

assert isinstance(mo, re.Match)
assert mo.start() == 1
assert mo.end() == 5
