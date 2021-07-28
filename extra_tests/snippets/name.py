#when name.py is run __name__ should equal to __main__
assert __name__ == "__main__"

from import_name import import_func

#__name__ should be set to import_func
import_func()

assert __name__ == "__main__"

assert __builtins__.__name__ == "builtins"
import builtins
assert builtins.__name__ == "builtins"
