from reprlib import recursive_repr as _recursive_repr

class defaultdict(dict):
    def __init__(self, *args, **kwargs):
        if len(args) >= 1:
            default_factory = args[0]
            if default_factory is not None and not callable(default_factory):
                raise TypeError("first argument must be callable or None")
            args = args[1:]
        else:
            default_factory = None
        super().__init__(*args, **kwargs)
        self.default_factory = default_factory

    def __missing__(self, key):
        if self.default_factory is not None:
            val = self.default_factory()
        else:
            raise KeyError(key)
        self[key] = val
        return val

    @_recursive_repr()
    def __repr_factory(factory):
        return repr(factory)

    def __repr__(self):
        return f"{type(self).__name__}({defaultdict.__repr_factory(self.default_factory)}, {dict.__repr__(self)})"

    def copy(self):
        return type(self)(self.default_factory, self)

    __copy__ = copy

    def __reduce__(self):
        if self.default_factory is not None:
            args = self.default_factory,
        else:
            args = ()
        return type(self), args, None, None, iter(self.items())

defaultdict.__module__ = 'collections'
