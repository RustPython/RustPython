from . import runtime


def continuation(*_args, **_kwargs):
    def decorator(func):
        return func
    return decorator


class ActorRef:
    def __init__(self, address):
        self.address = address

    def async_call(self, method, *args, **kwargs):
        if runtime.mode() != "checkpoint":
            raise RuntimeError("actor async is only supported in checkpoint mode without FSM")
        return _ActorAwaitable(self.address, method, *args, **kwargs)


class _ActorAwaitable:
    def __init__(self, address, method, *args, **kwargs):
        self.address = address
        self.method = method
        self.args = args
        self.kwargs = kwargs

    def __await__(self):
        raise RuntimeError("actor await not implemented")
