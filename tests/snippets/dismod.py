import dis

dis.disassemble(compile("5 + x + 5 or 2", "", "eval"))
print("\n")
dis.disassemble(compile("def f(x):\n   return 1", "", "exec"))
print("\n")
dis.disassemble(compile("if a:\n 1 or 2\nelif x == 'hello':\n 3\nelse:\n 4", "", "exec"))
print("\n")
dis.disassemble(compile("f(x=1, y=2)", "", "eval"))
print("\n")
