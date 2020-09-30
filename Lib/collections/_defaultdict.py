class defaultdict(dict):
    def __init__(self, *args, **kwargs):
        if len(args) >= 1:
            default_factory = args[0]
            args = args[1:]
        else:
            default_factory = None
        super().__init__(*args, **kwargs)
        self.default_factory = default_factory

    def __missing__(self, key):
        if self.default_factory:
            return self.default_factory()
        else:
            raise KeyError(key)

    def __repr__(self):
        return f"defaultdict({self.default_factory}, {dict.__repr__(self)})"

