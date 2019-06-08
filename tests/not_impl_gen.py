objects = [
    bool,
    bytearray,
    bytes,
    complex,
    dict,
    float,
    frozenset,
    int,
    list,
    memoryview,
    range,
    set,
    str,
    tuple,
    object,
]


def attr_is_not_inherited(type_, attr):
    """
    returns True if type_'s attr is not inherited from any of its base classes
    """

    bases = obj.__mro__[1:]

    return getattr(obj, attr) not in (
        getattr(base, attr, None) for base in bases)


header = open("generator/not_impl_header.txt")
footer = open("generator/not_impl_footer.txt")
output = open("snippets/whats_left_to_implement.py", "w")

output.write(header.read())
output.write("expected_methods = {\n")

for obj in objects:
    output.write(f" '{obj.__name__}': ({obj.__name__}, [\n")
    output.write("\n".join(
        f"  {attr!r},"
        for attr in dir(obj)
        if attr_is_not_inherited(obj, attr)
    ))
    output.write("\n ])," + ("\n" if objects[-1] == obj else "\n\n"))

output.write("}\n\n")
output.write(footer.read())
