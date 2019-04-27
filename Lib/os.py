from _os import *

curdir = '.'
pardir = '..'
extsep = '.'

if name == 'nt':
    sep = '\\'
    linesep = '\r\n'
    altsep = '/'
    pathsep = ';'
else:
    sep = '/'
    linesep = '\n'
    altsep = None
    pathsep = ':'

# Change environ to automatically call putenv(), unsetenv if they exist.
from _collections_abc import MutableMapping

class _Environ(MutableMapping):
    def __init__(self, data, encodekey, decodekey, encodevalue, decodevalue, putenv, unsetenv):
        self.encodekey = encodekey
        self.decodekey = decodekey
        self.encodevalue = encodevalue
        self.decodevalue = decodevalue
        self.putenv = putenv
        self.unsetenv = unsetenv
        self._data = data

    def __getitem__(self, key):
        try:
            value = self._data[self.encodekey(key)]
        except KeyError:
            # raise KeyError with the original key value
            # raise KeyError(key) from None
            raise KeyError(key)

        return self.decodevalue(value)

    def __setitem__(self, key, value):
        key = self.encodekey(key)
        value = self.encodevalue(value)
        self.putenv(key, value)
        self._data[key] = value

    def __delitem__(self, key):
        encodedkey = self.encodekey(key)
        self.unsetenv(encodedkey)
        try:
            del self._data[encodedkey]
        except KeyError:
            # raise KeyError with the original key value
            # raise KeyError(key) from None
            raise KeyError(key)

    def __iter__(self):
        # list() from dict object is an atomic operation
        keys = list(self._data)
        for key in keys:
            yield self.decodekey(key)

    def __len__(self):
        return len(self._data)

    def __repr__(self):
        return 'environ({{{}}})'.format(', '.join(
            ('{!r}: {!r}'.format(self.decodekey(key), self.decodevalue(value))
            for key, value in self._data.items())))

    def copy(self):
        return dict(self)

    def setdefault(self, key, value):
        if key not in self:
            self[key] = value
        return self[key]

try:
    _putenv = putenv
except NameError:
    _putenv = lambda key, value: None
# else:
#     if "putenv" not in __all__:
#         __all__.append("putenv")

try:
    _unsetenv = unsetenv
except NameError:
    _unsetenv = lambda key: _putenv(key, "")
# else:
#     if "unsetenv" not in __all__:
#         __all__.append("unsetenv")

def _createenviron():
    # if name == 'nt':
    #     # Where Env Var Names Must Be UPPERCASE
    #     def check_str(value):
    #         if not isinstance(value, str):
    #             raise TypeError("str expected, not %s" % type(value).__name__)
    #         return value
    #     encode = check_str
    #     decode = str
    #     def encodekey(key):
    #         return encode(key).upper()
    #     data = {}
    #     for key, value in environ.items():
    #         data[encodekey(key)] = value
    # else:
    #     # Where Env Var Names Can Be Mixed Case
    #     encoding = sys.getfilesystemencoding()
    #     def encode(value):
    #         if not isinstance(value, str):
    #             raise TypeError("str expected, not %s" % type(value).__name__)
    #         return value.encode(encoding, 'surrogateescape')
    #     def decode(value):
    #         return value.decode(encoding, 'surrogateescape')
    #     encodekey = encode
    decode = str
    encode = str
    encodekey = encode
    data = environ
    return _Environ(data,
        encodekey, decode,
        encode, decode,
        _putenv, _unsetenv)

# unicode environ
environ = _createenviron()
del _createenviron


def getenv(key, default=None):
    """Get an environment variable, return None if it doesn't exist.
    The optional second argument can specify an alternate default.
    key, default and the result are str."""
    return environ.get(key, default)
