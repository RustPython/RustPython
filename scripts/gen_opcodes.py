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


def group_ranges(it: "Iterable[int]") -> "Iterator[range]":
    """
    Group consecutive numbers into ranges.

    Parameters
    ----------
    it : Iterable[int]
        Numbers to group into ranges.

    Notes
    -----
    Numbers in `it` must be sorted in ascending order.

    Examples
    --------
    >>> nums = [0, 1, 2, 3, 17, 18, 42, 50, 51]
    >>> list(group_ranges(nums))
    [range(0, 4), range(17, 19), range(42, 43), range(50, 52)]
    """
    nums = list(it)
    start = prev = nums[0]
    for num in nums[1:] + [None]:
        if num is None or num != prev + 1:
            yield range(start, prev + 1)
            start = num
        prev = num


def fmt_ranges(ids: "Iterable[range]", *, min_length: int = 3) -> str:
    """
    Get valid opcode ranges in Rust's `match` syntax.

    Parameters
    ----------
    ids : Iterable[range]
        Ranges to be formatted.
    min_length : int, default 3
        Minimum range length, if a range is less than this it will be expanded.

    Examples
    --------
    >>> ids = [range(10, 11), range(20, 22), range(30, 33)]

    >>> fmt_ranges(ids)
    10 | 20 | 21 | 30..=32

    >>> fmt_ranges(ids, min_length=2)
    10 | 20..=21 | 30..=32
    """
    return " | ".join(
        " | ".join(r) if len(r) < min_length else f"{r.start}..={r.stop - 1}"
        for r in ids
    )


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

    @property
    def fn_new_unchecked(self) -> str:
        return f"""
/// Creates a new `{self.enum_name}` without checking the value is a valid opcode ID.
///
/// # Safety
///
/// The caller must ensure that `id` satisfies `{self.enum_name}::is_valid(id)`.
#[must_use]
pub const unsafe fn new_unchecked(id: {self.typ}) -> Self {{
    // SAFETY: caller responsibility
    unsafe {{ std::mem::transmute::<{self.typ}, Self>(id) }}
}}
"""

    @property
    def fn_is_valid(self) -> str:
        valid_ranges = fmt_ranges(
            group_ranges(sorted(self._analysis.opmap[inst.name] for inst in self))
        )
        return f"""
/// Whether the given ID matches one of the opcode IDs.
#[must_use]
pub const fn is_valid(id: {self.typ}) -> bool {{
    matches!(id, {valid_ranges})
}}
        """

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
