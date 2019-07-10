
import io
import sys

f = io.StringIO()
sys.stderr = f

import logging

logging.error('WOOT')
logging.warning('WARN')

print(f.getvalue())

