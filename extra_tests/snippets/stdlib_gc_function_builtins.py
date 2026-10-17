"""Functions must report every owned builtins reference to the cycle collector."""

import gc
import weakref


def check_function_cycle(shared_namespace):
    namespace = {}
    builtins = namespace if shared_namespace else {}
    namespace["__builtins__"] = builtins
    exec("def temporary(): return 42", namespace)
    keeper = namespace["temporary"]
    builtins["temporary"] = keeper
    reference = weakref.ref(keeper)
    del namespace, builtins

    gc.collect()
    assert reference() is keeper
    assert keeper() == 42

    del keeper
    gc.collect()
    assert reference() is None


check_function_cycle(shared_namespace=False)
check_function_cycle(shared_namespace=True)
print("ok")
