class Foo:
    ABC = 1

    def __init__(self):
        super().__init__()

    def bar(self):
        pass

    @classmethod
    def bar_2(cls):
        pass
