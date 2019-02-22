import dis

dis.dis(compile("5 + x + 5 or 2", "", "eval"))
print("\n")
dis.dis(compile("def f(x):\n   return 1", "", "exec"))
print("\n")
dis.dis(compile("if a:\n 1 or 2\nelif x == 'hello':\n 3\nelse:\n 4", "", "exec"))
print("\n")
dis.dis(compile("f(x=1, y=2)", "", "eval"))
print("\n")

def f():
    with g():
        try:
            for a in {1: 4, 2: 5}:
                yield [True and False or True, []]
        except Exception:
            raise not ValueError({1 for i in [1,2,3]})

dis.dis(f)
