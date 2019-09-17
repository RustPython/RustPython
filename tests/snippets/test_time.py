

import time

x = time.gmtime(1000)

assert x.tm_year == 1970
assert x.tm_min == 16
assert x.tm_sec == 40
assert x.tm_isdst == 0

s = time.strftime('%Y-%m-%d-%H-%M-%S', x)
# print(s)
assert s == '1970-01-01-00-16-40'

x2 = time.strptime(s, '%Y-%m-%d-%H-%M-%S')
assert x2.tm_min == 16

s = time.asctime(x)
# print(s)
assert s == 'Thu Jan  1 00:16:40 1970'

