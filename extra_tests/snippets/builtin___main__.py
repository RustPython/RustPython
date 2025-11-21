import sys

main_module = sys.modules["__main__"]
assert main_module.__file__.endswith("builtin___main__.py")
assert main_module.__cached__ is None
