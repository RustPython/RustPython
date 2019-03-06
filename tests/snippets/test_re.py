
import re

haystack = "Hello world"
needle = 'ello'

mo = re.search(needle, haystack)
print(mo)

# Does not work on python 3.6:
# assert isinstance(mo, re.Match)
assert mo.start() == 1
assert mo.end() == 5
