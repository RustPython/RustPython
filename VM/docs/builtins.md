Byterun

* Builtins are exposted to frame.f_builtins
* f_builtins is assigned during frame creation,
    self.f_builtins = f_locals['__builtins__']
    if hasattr(self.f_builtins, '__dict__'):
      self.f_builtins = self.f_builtins.__dict__
* f_locals has a __`____builtins___` field which is directly the `__builtins__` module


Jaspy

* The `module()` function creates either a NativeModule or PythonModule
* The objects in the module are PyType.native
* The function call is abstracted as a `call` function, which handles different

* IMPORT_NAME depends on `__import__()` in builtins

TODO:

* Implement a new type NativeFunction
* Wrap a function pointer in NativeFunction
* Refactor the CALL_FUNCTION case so it can call both python function and native function
* During frame creation, force push a nativefunction `print` into the namespace
* Modify LOAD_* so they can search for names in builtins

* Create a module type
* In VM initialization, load the builtins module into locals
* During frame creation, create a field that conatins the builtins dict

