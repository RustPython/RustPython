class Foo:
    print("Defining class")
    def __init__(self):
        print("initing: ", self)
        self.x = 5

    y = 7

print("Done defining: ", Foo)
print("Init: ", Foo.__init__)
print("y = ", Foo.y)
foo = Foo()
print("Done initting: ", foo)
print("Foo's x: ", foo.x)
