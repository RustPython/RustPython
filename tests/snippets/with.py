

class ContextManager:
    def __enter__(self):
        print('Entrada')
        ls.append(1)
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        ls.append(2)
        print('Wiedersehen')

    def __str__(self):
        ls.append(3)
        return "c'est moi!"

ls = []
with ContextManager() as c:
    print(c)
assert ls == [1, 3, 2]

class ContextManager2:
    def __enter__(self):
        print('Ni hau')
        ls.append(4)
        return ls

    def __exit__(self, exc_type, exc_val, exc_tb):
        ls.append(5)
        print('Ajuus')

ls = []
with ContextManager2() as c:
    print(c)
    assert c == [4]
assert ls == [4, 5]

ls = []
with ContextManager() as c1, ContextManager2() as c2:
    print(c1)
    assert c2 == [1, 4, 3]
assert ls == [1, 4, 3, 5, 2]
