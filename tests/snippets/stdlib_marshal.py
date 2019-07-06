import marshal
orig = compile("1 + 1", "", 'eval')

dumped = marshal.dumps(orig)
loaded = marshal.loads(dumped)

assert eval(loaded) == eval(orig)
