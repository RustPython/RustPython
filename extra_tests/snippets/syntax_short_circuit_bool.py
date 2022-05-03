# Python carefully avoids evaluating bools more than once in a variety of situations.
# Eg:
# In the statement
#  if a or b:
# it doesn't simply compute (a or b) and then evaluate the result to decide whether to
# jump. If a is true it jumps directly to the body of the if statement.
# We can confirm that this behaviour is correct in python code.


# A Bool that raises an exception if evaluated twice!
class ExplodingBool():
    def __init__(self, value):
        self.value = value
        self.booled = False

    def __bool__(self):
        assert not self.booled
        self.booled = True
        return self.value

y = (ExplodingBool(False) and False and True and False)
print(y)

if (ExplodingBool(True) or False or True or False):
    pass

assert ExplodingBool(True) or False

while ExplodingBool(False) and False:
    pass

# if ExplodingBool(False) and False and True and False:
#     pass
