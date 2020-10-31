from _ctypes import dlopen, dlsym, CFuncPtr

class LibraryLoader:
    def __init__(self, dll_path):
        self._dll_path = dll_path
        self._dll = dlopen(dll_path)

    def __getattr__(self, attr):
        if attr.startswith('_'):
            raise AttributeError(attr)
        func_ptr = dlsym(self._dll, attr)
        if not func_ptr:
            raise AttributeError("{}: undefined symbol: {}".format(self._dll_path, attr))

        return lambda: func_ptr.call()


cdll = LibraryLoader
