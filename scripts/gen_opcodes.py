#!/usr/bin/env python
import abc
import collections
import itertools
import pathlib
import re
import subprocess  # for `cargo fmt`
import sys
import typing

if typing.TYPE_CHECKING:
    from collections.abc import Iterable, Iterator

CPYTHON_PATH = (
    pathlib.Path(__file__).parents[2] / "cpython"  # Local filesystem path of cpython
)

_cases_generator_path = CPYTHON_PATH / "Tools" / "cases_generator"
sys.path.append(str(_cases_generator_path))


import analyzer
from generators_common import DEFAULT_INPUT
from stack import StackOffset, get_stack_effect

ROOT = pathlib.Path(__file__).parents[1]
OUT_PATH = ROOT / "compiler" / "core" / "src" / "opcodes.rs"

DERIVE = "#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromPrimitive)]"


def _var_size(var):
    """
    Adapted from https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Tools/cases_generator/stack.py#L24-L36
    """
    if var.condition:
        if var.condition == "0":
            return "0"
        elif var.condition == "1":
            return var.size
        elif var.condition == "oparg & 1" and var.size == "1":
            return f"({var.condition})"
        else:
            return f"(if {var.condition} {{ {var.size} }} else {{ 0 }})"
    else:
        return var.size


StackOffset.pop = lambda self, item: self.popped.append(_var_size(item))
StackOffset.push = lambda self, item: self.pushed.append(_var_size(item))


def enum_variant_name(name: str) -> str:
    return name.title().replace("_", "")


class InstructionsMeta(metaclass=abc.ABCMeta):
    def __init__(self, analysis: analyzer.Analysis) -> None:
        self._analysis = analysis

    @abc.abstractmethod
    def __iter__(
        self,
    ) -> "Iterator[analyzer.Instruction | analyzer.PseudoInstruction]": ...

    @property
    @abc.abstractmethod
    def typ(self) -> str:
        """
        Opcode ID type (u8/u16/u32/etc)
        """
        ...

    @property
    @abc.abstractmethod
    def enum_name(self) -> str: ...

    @property
    def rust_code(self) -> str:
        enum_variant_defs = ",\n".join(
            f"{inst.name} = {self._analysis.opmap[inst.name]}" for inst in self
        )
        funcs = "\n\n".join(
            getattr(self, attr).strip()
            for attr in sorted(dir(self))
            if attr.startswith("fn_")
        )

        return f"""
{DERIVE}
#[num_enum(error_type(name = MarshalError, constructor = new_invalid_bytecode))]
#[repr({self.typ})]
pub enum {self.enum_name} {{
{enum_variant_defs}
}}

impl {self.enum_name} {{
{funcs}
}}
        """.strip()

    def build_has_attr_fn(self, fn_attr: str, prop_attr: str, doc_flag: str):
        matches = "|".join(
            f"Self::{inst.name}" for inst in self if getattr(inst.properties, prop_attr)
        )
        if matches:
            inner = f"matches!(*self, {matches})"
        else:
            inner = "false"

        return f"""
/// Whether opcode ID have '{doc_flag}' set.
#[must_use]
pub const fn has_{fn_attr}(&self) -> bool {{
{inner}
}}
        """

    fn_has_arg = property(
        lambda self: self.build_has_attr_fn("arg", "oparg", "HAS_ARG_FLAG")
    )
    fn_has_const = property(
        lambda self: self.build_has_attr_fn("const", "uses_co_consts", "HAS_CONST_FLAG")
    )
    fn_has_name = property(
        lambda self: self.build_has_attr_fn("name", "uses_co_names", "HAS_NAME_FLAG")
    )
    fn_has_jump = property(
        lambda self: self.build_has_attr_fn("jump", "jumps", "HAS_JUMP_FLAG")
    )
    fn_has_free = property(
        lambda self: self.build_has_attr_fn("free", "has_free", "HAS_FREE_FLAG")
    )
    fn_has_local = property(
        lambda self: self.build_has_attr_fn("local", "uses_locals", "HAS_LOCAL_FLAG")
    )
    fn_has_exc = property(
        lambda self: self.build_has_attr_fn("exc", "pure", "HAS_PURE_FLAG")
    )


class RealInstructions(InstructionsMeta):
    enum_name = "RealOpcode"
    typ = "u8"

    def __iter__(self) -> "Iterator[analyzer.Instruction | analyzer.PseudoInstruction]":
        yield from sorted(
            itertools.chain(
                self._analysis.instructions.values(),
                [analyzer.Instruction("INSTRUMENTED_LINE", [], None)],
            ),
            key=lambda inst: inst.name,
        )

    def _generate_stack_effect(self, direction: str) -> str:
        """
        Adapted from https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Tools/cases_generator/stack.py#L89-L111
        """

        lines = []
        for inst in self:
            stack = get_stack_effect(inst)
            if direction == "popped":
                val = -stack.base_offset
            elif direction == "pushed":
                val = stack.top_offset - stack.base_offset

            expr = val.to_c()
            line = f"Self::{inst.name} => {expr}"
            lines.append(line)

        branches = ",\n".join(lines)
        doc = "from" if direction == "popped" else "on"
        return f"""
/// How many items should be {direction} {doc} the stack.
pub const fn num_{direction}(&self, oparg: i32) -> i32 {{
    match *self {{
{branches}
    }}
}}
"""

    @property
    def fn_num_popped(self) -> str:
        return self._generate_stack_effect("popped")

    @property
    def fn_num_pushed(self) -> str:
        return self._generate_stack_effect("pushed")

    @property
    def fn_deopt(self) -> str:
        def format_deopt_variants(lst: list[str]) -> str:
            return "|".join(f"Self::{v}" for v in lst)

        deopts = collections.defaultdict(list)
        for inst in self:
            deopt = inst.name

            if inst.family is not None:
                deopt = inst.family.name

            if inst.name == deopt:
                continue
            deopts[deopt].append(inst.name)

        branches = ",\n".join(
            f"{format_deopt_variants(deopt)} => Self::{name}"
            for name, deopt in sorted(deopts.items())
        )
        return f"""
pub const fn deopt(&self) -> Option<Self> {{
    Some(match *self {{
{branches},
_ => return None,
    }})
}}
""".strip()


class PseudoInstructions(InstructionsMeta):
    enum_name = "PseudoOpcode"
    typ = "u16"

    def __iter__(self) -> "Iterator[analyzer.PseudoInstruction]":
        yield from sorted(self._analysis.pseudos.values(), key=lambda inst: inst.name)


def main():
    analysis = analyzer.analyze_files([DEFAULT_INPUT])
    real_instructions = RealInstructions(analysis)
    pseudo_instructions = PseudoInstructions(analysis)

    script_path = pathlib.Path(__file__).absolute().relative_to(ROOT).as_posix()
    out = f"""
//! Python opcode implementation. Currently aligned with cpython 3.13.7

// This file is generated by {script_path}
// Do not edit!

use crate::marshal::MarshalError;
use num_enum::TryFromPrimitive;

{real_instructions.rust_code}

{pseudo_instructions.rust_code}

const fn new_invalid_bytecode<T: num_traits::int::PrimInt>(_: T) -> MarshalError {{
  MarshalError::InvalidBytecode
}}
    """.strip()

    replacements = {name: enum_variant_name(name) for name in analysis.opmap}
    inner_pattern = "|".join(replacements)
    pattern = re.compile(rf"\b({inner_pattern})\b")
    out = pattern.sub(lambda m: replacements[m.group(0)], out)
    OUT_PATH.write_text(out)
    print("Running `cargo fmt`")
    subprocess.run(["cargo", "fmt"], cwd=ROOT)


if __name__ == "__main__":
    main()
