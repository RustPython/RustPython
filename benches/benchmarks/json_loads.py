import json

with open('benches/_data/pypi_org__simple__psutil.json') as f:
    data = f.read()


loaded = json.loads(data)
