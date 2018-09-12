
ls = []

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

with ContextManager() as c:
    print(c)

assert ls == [1, 3, 2]

ls = []
class ContextManager2:
    def __enter__(self):
        print('Entrada')
        ls.append(1)
        return ls

    def __exit__(self, exc_type, exc_val, exc_tb):
        ls.append(2)
        print('Wiedersehen')

with ContextManager2() as c:
    print(c)
    assert c == [1]

assert ls == [1, 2]
