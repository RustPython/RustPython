"""Regression tests for inspect.getsource returning full source code."""
import inspect


def a(
    a=0,
):
    pass


source = inspect.getsource(a)
assert "def" in source, f"Expected full source, got: {source!r}"

# Ensure the full definition including the `def` line is present
assert source.startswith("def a("), f"Source should start with 'def a(', got: {source!r}"


# Async function with multi-line parameters
async def b(
    x=1,
    y=2,
):
    pass


source_b = inspect.getsource(b)
assert "async def" in source_b, f"Expected 'async def' in source, got: {source_b!r}"


# Function with keyword-only defaults
def c(
    *,
    kw=42,
):
    pass


source_c = inspect.getsource(c)
assert "def" in source_c, f"Expected full source for kw-only defaults, got: {source_c!r}"
assert source_c.startswith("def c("), f"Source should start with 'def c(', got: {source_c!r}"
