"""Generate public _opcode descriptors without expanding executable opcodes."""

import argparse
import ast
import os
import re
import subprocess
from pathlib import Path

from target_source import verify_source

ROOT = Path(__file__).resolve().parents[2]


def expression(text):
    """Translate the pinned grammar with an explicit panic-free i32 wrapping policy."""

    def emit(node):
        if isinstance(node, ast.Constant) and type(node.value) is int:
            return f"{node.value}i32"
        if isinstance(node, ast.Name) and node.id == "oparg":
            return "oparg"
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
            return f"({emit(node.operand)}).wrapping_neg()"
        if isinstance(node, ast.BinOp):
            left, right = emit(node.left), emit(node.right)
            methods = {
                ast.Add: "wrapping_add",
                ast.Sub: "wrapping_sub",
                ast.Mult: "wrapping_mul",
            }
            if method := methods.get(type(node.op)):
                # The argument list already delimits this entire bit expression.
                if isinstance(node.right, ast.BinOp) and isinstance(
                    node.right.op, (ast.BitAnd, ast.RShift)
                ):
                    right = right[1:-1]
                return f"({left}).{method}({right})"
            if isinstance(node.op, ast.BitAnd):
                return f"({left} & {right})"
            if (
                isinstance(node.op, ast.RShift)
                and isinstance(node.right, ast.Constant)
                and 0 <= node.right.value < 32
            ):
                return f"({left} >> {node.right.value})"
        raise ValueError(f"unsupported pinned stack expression: {text}")

    return emit(ast.parse(text, mode="eval").body)


def generate(source):
    verify_source(source)
    ids = {
        name: int(value)
        for name, value in re.findall(
            r"^#define\s+(\w+)\s+(\d+)\s*$",
            (source / "Include/opcode_ids.h").read_text(),
            re.M,
        )
    }
    header = (source / "Include/internal/pycore_opcode_metadata.h").read_text()
    extent, body = re.search(
        r"const struct opcode_metadata _PyOpcode_opcode_metadata\[(\d+)\] = \{(.*?)\n\};",
        header,
        re.S,
    ).groups()
    assert int(extent) == 267
    flag_values = {
        name: int(value)
        for name, value in re.findall(
            r"^#define (HAS_\w+_FLAG) \((\d+)\)$", header, re.M
        )
    }
    descriptors = {}
    for name, flags in re.findall(r"\[(\w+)\] = \{ true, (?:\w+|-1), (.*?) \},", body):
        value = 0
        for flag in flags.split(" | "):
            value |= 0 if flag == "0" else flag_values[flag]
        descriptors[ids[name]] = (name, value)
    assert len(descriptors) == 245
    deopt_body = re.search(
        r"const uint8_t _PyOpcode_Deopt\[256\] = \{(.*?)\n\};", header, re.S
    )[1]
    number = lambda value: int(value) if value.isdigit() else ids[value]
    deopt = {
        number(a): number(b) for a, b in re.findall(r"\[(\w+)\] = (\w+),", deopt_body)
    }
    assert set(deopt) == set(range(256))
    utils = (source / "Include/internal/pycore_opcode_utils.h").read_text()
    block = re.search(
        r"#define IS_BLOCK_PUSH_OPCODE\(opcode\)(.*?)(?=\n\n)", utils, re.S
    )[1]
    block_ids = {ids[name] for name in re.findall(r"opcode\) == (\w+)", block)}
    assert block_ids == {
        ids[x] for x in ("SETUP_FINALLY", "SETUP_WITH", "SETUP_CLEANUP")
    }
    effects = []
    for kind in ("popped", "pushed"):
        body = re.search(
            rf"int _PyOpcode_num_{kind}\(int opcode, int oparg\)  \{{(.*?)\n\}}",
            header,
            re.S,
        )[1]
        rows = {
            ids[name]: expression(expr)
            for name, expr in re.findall(r"case (\w+):\s*return (.*?);", body)
        }
        assert set(rows) == set(descriptors)
        effects.append(rows)
    rows = []
    for opcode in range(267):
        if opcode not in descriptors:
            rows.append("0,")
            continue
        name, flags = descriptors[opcode]
        value = flags | (1 << 31)
        if opcode < 256 and deopt[opcode] != opcode:
            value |= 1 << 30
        if opcode in block_ids:
            value |= 1 << 29
        rows.append(f"{value}, // {opcode}: {name}")
    methods = "\n".join(
        f"#[must_use] pub const fn {name}(self) -> bool {{ self.flags & {flag_values[flag]} != 0 }}"
        for name, flag in (
            ("has_arg", "HAS_ARG_FLAG"),
            ("has_const", "HAS_CONST_FLAG"),
            ("has_name", "HAS_NAME_FLAG"),
            ("has_jump", "HAS_JUMP_FLAG"),
            ("has_free", "HAS_FREE_FLAG"),
            ("has_local", "HAS_LOCAL_FLAG"),
        )
    )
    arms = "\n".join(
        f"{op} => ({effects[0][op]}, {effects[1][op]}), // {name}"
        for op, (name, _) in sorted(descriptors.items())
        if not (op < 256 and deopt[op] != op)
    )
    return f"""// Generated by tools/opcode_metadata/generate_public_metadata.py from pinned final CPython.
//! Public introspection metadata, independent of the VM's executable opcode set.
//! Describing an opcode does not enable execution of its CPython specialization.
const DESCRIPTORS: [u32; 267] = [{chr(10).join(rows)}\n];
#[derive(Clone, Copy)]
pub struct PublicOpcode {{ opcode: u16, flags: u32 }}
impl PublicOpcode {{
    #[must_use]
    pub fn from_i32(opcode: i32) -> Option<Self> {{
        let index = usize::try_from(opcode).ok()?;
        let flags = *DESCRIPTORS.get(index)?;
        (flags & (1 << 31) != 0).then_some(Self {{ opcode: u16::try_from(index).ok()?, flags }})
    }}
    {methods}
    #[must_use] pub const fn is_block_push(self) -> bool {{ self.flags & (1 << 29) != 0 }}
    /// Existing caller conversion is intentionally separate from this metadata domain.
    #[must_use]
    pub fn stack_effect(self, oparg: u32, jump: Option<bool>) -> Option<i32> {{
        if self.flags & (1 << 30) != 0 {{ return None; }}
        let oparg = oparg.cast_signed();
        let (popped, pushed) = match self.opcode {{ {arms}\n _ => return None }};
        if popped < 0 || pushed < 0 {{ return None; }}
        if self.is_block_push() && jump == Some(false) {{ return Some(0); }}
        Some(pushed.wrapping_sub(popped))
    }}
}}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=os.environ.get("CPYTHON_ROOT"))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.source is None:
        parser.error("--source or CPYTHON_ROOT is required")
    output = subprocess.check_output(
        ["rustfmt", "--emit=stdout", "--edition=2024"],
        input=generate(args.source),
        text=True,
    )
    target = ROOT / "crates/compiler-core/src/bytecode/public_metadata.rs"
    if args.check:
        assert target.read_text() == output
    else:
        target.write_text(output)


if __name__ == "__main__":
    main()
