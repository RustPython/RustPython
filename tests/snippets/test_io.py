

import io

f = io.StringIO()
f.write('bladibla')
assert f.getvalue() == 'bladibla'

# TODO:
# print('fubar', file=f, end='')
print(f.getvalue())

# TODO:
# assert f.getvalue() == 'bladiblafubar'
