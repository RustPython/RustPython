assert type.__module__ == 'builtins'
assert type.__qualname__ == 'type'
assert type.__name__ == 'type'
assert isinstance(type.__doc__, str)

import builtins
assert builtins.iter.__class__.__module__ == 'builtins'
