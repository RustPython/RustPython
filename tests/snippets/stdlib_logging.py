
import io
import sys

f = io.StringIO()
sys.stderr = f

import logging

logging.error('WOOT')
logging.warning('WARN')

res = f.getvalue()

assert  'WOOT' in res
assert  'WARN' in res
print(res)

