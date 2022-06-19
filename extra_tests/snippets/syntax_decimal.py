try:
    eval("0.E")
except SyntaxError:
   pass
else:
  assert False
