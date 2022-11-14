from rust_py_module import RustStruct, rust_function

class PythonPerson:
    def __init__(self, name):
        self.name = name

def python_callback():
    python_person = PythonPerson("Peter Python")
    rust_object = rust_function(42, "This is a python string", python_person)
    rust_object.print_in_rust_from_python()

def take_string(string):
    print("Calling python function from rust with string: " + string)
