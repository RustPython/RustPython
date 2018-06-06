import bytecode
import sys
import json
import types


class CodeEncoder(json.JSONEncoder):
    def default(self, obj):
        if (isinstance(obj, types.CodeType)):
            return serialize_code(obj)
        return json.JSONEncoder.default(self, obj)

def serialize_code(code):
    c = bytecode.Bytecode().from_code(code).to_concrete_bytecode()
    return (
        {
            "co_consts": consts_to_rust_enum(c.consts),
            "co_names": c.names,
            "co_name": c.name,
            "co_code": parse_co_code_to_str(c),
            "co_varnames": c.varnames
        }
    )


def consts_to_rust_enum(consts):
    def capitalize_first(s):
        return s[0].upper() + s[1:]

    def const_to_rust_enum(const):
        if type(const).__name__ == "tuple":
            return {capitalize_first(str(type(const).__name__)): list(map(const_to_rust_enum, const))}
        else:
            return {capitalize_first(str(type(const).__name__)): const}
    return list(map(const_to_rust_enum, consts))


def parse_co_code_to_str(c):
    return list(
        map(lambda op: (op.size, op.name, op.arg if op.arg != bytecode.UNSET else None),
            c)
    )


def main():

    filename = sys.argv[1]
    with open(filename, 'rU') as f:
        code = f.read()

    code = compile(code, filename, "exec")

    print(CodeEncoder().encode(code))

if __name__ == "__main__":
    main()
