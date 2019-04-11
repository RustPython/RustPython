from _os import *

class _Environ():
    def __init__(self, data, putenv, unsetenv):
        self.putenv = putenv
        self.unsetenv = unsetenv
        self._data = data

    def __getitem__(self, key):
        return self._data[key]

    def __setitem__(self, key, value):
        self.putenv(key, value)
        self._data[key] = value

    def __delitem__(self, key):
        self.unsetenv(key)
        del self._data[key]

    def __iter__(self):
        # list() from dict object is an atomic operation
        keys = list(self._data)
        for key in keys:
            yield key

    def __len__(self):
        return len(self._data)

    def __repr__(self):
        return 'environ({{{}}})'.format(', '.join(
            ('{}: {}'.format(key, value)
            for key, value in self._data.items())))

    def copy(self):
        return dict(self)

    def setdefault(self, key, value):
        if key not in self:
            self[key] = value
        return self[key]

environ = _Environ(environ, putenv, unsetenv)

def getenv(key, default=None):
    """Get an environment variable, return None if it doesn't exist.
    The optional second argument can specify an alternate default.
    key, default and the result are str."""
    try:
        return environ[key]
    except KeyError:
        return default
