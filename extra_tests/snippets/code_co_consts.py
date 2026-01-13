"""
Test co_consts behavior for Python 3.14+

In Python 3.14+:
- Functions with docstrings have the docstring as co_consts[0]
- CO_HAS_DOCSTRING flag (0x4000000) indicates docstring presence
- Functions without docstrings do NOT have None added as placeholder for docstring

Note: Other constants (small integers, code objects, etc.) may still appear in co_consts
depending on optimization level. This test focuses on docstring behavior.
"""


# Test function with docstring - docstring should be co_consts[0]
def with_doc():
    """This is a docstring"""
    return 1


assert with_doc.__code__.co_consts[0] == "This is a docstring", (
    with_doc.__code__.co_consts
)
assert with_doc.__doc__ == "This is a docstring"
# Check CO_HAS_DOCSTRING flag (0x4000000)
assert with_doc.__code__.co_flags & 0x4000000, hex(with_doc.__code__.co_flags)


# Test function without docstring - should NOT have HAS_DOCSTRING flag
def no_doc():
    return 1


assert not (no_doc.__code__.co_flags & 0x4000000), hex(no_doc.__code__.co_flags)
assert no_doc.__doc__ is None


# Test async function with docstring
from asyncio import sleep


async def async_with_doc():
    """Async docstring"""
    await sleep(1)
    return 1


assert async_with_doc.__code__.co_consts[0] == "Async docstring", (
    async_with_doc.__code__.co_consts
)
assert async_with_doc.__doc__ == "Async docstring"
assert async_with_doc.__code__.co_flags & 0x4000000


# Test async function without docstring
async def async_no_doc():
    await sleep(1)
    return 1


assert not (async_no_doc.__code__.co_flags & 0x4000000)
assert async_no_doc.__doc__ is None


# Test generator with docstring
def gen_with_doc():
    """Generator docstring"""
    yield 1
    yield 2


assert gen_with_doc.__code__.co_consts[0] == "Generator docstring"
assert gen_with_doc.__doc__ == "Generator docstring"
assert gen_with_doc.__code__.co_flags & 0x4000000


# Test generator without docstring
def gen_no_doc():
    yield 1
    yield 2


assert not (gen_no_doc.__code__.co_flags & 0x4000000)
assert gen_no_doc.__doc__ is None


# Test lambda - cannot have docstring
lambda_f = lambda: 0
assert not (lambda_f.__code__.co_flags & 0x4000000)
assert lambda_f.__doc__ is None


# Test class method with docstring
class cls_with_doc:
    def method():
        """Method docstring"""
        return 1


assert cls_with_doc.method.__code__.co_consts[0] == "Method docstring"
assert cls_with_doc.method.__doc__ == "Method docstring"


# Test class method without docstring
class cls_no_doc:
    def method():
        return 1


assert not (cls_no_doc.method.__code__.co_flags & 0x4000000)
assert cls_no_doc.method.__doc__ is None

print("All co_consts tests passed!")
