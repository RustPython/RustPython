from _ctypes import dlopen, dlsym

class LibraryLoader:
    def __init__(self, dll_path):
        self._dll_path = dll_path
        self._dll = dlopen(dll_path)

    def tt(self):
        dlsym(self._dll, "hello")


cdll = LibraryLoader
