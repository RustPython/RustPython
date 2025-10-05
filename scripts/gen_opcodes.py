#!/usr/bin/env python
import pathlib
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
from opcode_metadata_generator import cflags
from stack import StackOffset, get_stack_effect

ROOT = pathlib.Path(__file__).parents[1]
OUT_PATH = ROOT / "compiler" / "core" / "src" / "opcode.rs"


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


class Instruction:
    def __init__(
        self, ins: analyzer.Instruction, val: int, is_pseudo: bool = False
    ) -> None:
        self.id = val
        self.is_pseudo = is_pseudo
        self._inner = ins

    @property
    def name(self) -> str:
        return self._inner.name.title().replace("_", "")

    @property
    def flags(self) -> frozenset[str]:
        if not self.is_pseudo:
            return frozenset(cflags(self._inner.properties).split(" | "))

        flags = cflags(self._inner.properties)
        for flag in self._inner.flags:
            if flags == "0":
                flags = f"{flag}_FLAG"
            else:
                flags += f" | {flag}_FLAG"
        return frozenset(flags.split(" | "))

    @property
    def enum_variant(self) -> str:
        return f"Self::{self.name}"

    @property
    def enum_variant_def(self) -> str:
        return f"{self.name} = {self.id}"

    def __lt__(self, other) -> bool:
        return (self.is_pseudo, self._inner.name) < (other.is_pseudo, other._inner.name)


class Instructions:
    def __init__(self, analysis: analyzer.Analysis) -> None:
        inner = []
        for ins in analysis.instructions.values():
            inner.append(Instruction(ins, analysis.opmap[ins.name], is_pseudo=False))

        for pseudo in analysis.pseudos.values():
            inner.append(
                Instruction(pseudo, analysis.opmap[pseudo.name], is_pseudo=True)
            )
        self._inner = tuple(sorted(inner))

    def __iter__(self) -> "Iterator[analyzer.Instruction]":
        yield from self._inner

    def generate_enum_variant_defs(self) -> str:
        return ",\n".join(ins.enum_variant_def for ins in self)

    def generate_is_valid(self) -> str:
        valid_ranges = fmt_ranges(group_ranges(sorted(ins.id for ins in self)))
        return f"""
/// Whether the given ID matches one of the opcode IDs.
#[must_use]
pub const fn is_valid(id: u16) -> bool {{
    matches!(id, {valid_ranges})
}}
""".strip()

    def generate_is_pseudo(self) -> str:
        matches = " | ".join(ins.enum_variant for ins in self if ins.is_pseudo)
        return f"""
/// Whether opcode ID is pseudo.
#[must_use]
pub const fn is_pseudo(&self) -> bool {{
    matches!(*self, {matches})
}}
""".strip()

    def _generate_has_attr(self, attr: str, *, flag_override: str | None = None) -> str:
        flag = flag_override if flag_override else f"has_{attr}_flag".upper()
        matches = " | ".join(ins.enum_variant for ins in self if flag in ins.flags)
        return f"""
/// Whether opcode ID have '{flag}' set.
#[must_use]
pub const fn has_{attr}(&self) -> bool {{
    matches!(*self, {matches})
}}
""".strip()

    def generate_has_arg(self) -> str:
        return self._generate_has_attr("arg")

    def generate_has_const(self) -> str:
        return self._generate_has_attr("const")

    def generate_has_name(self) -> str:
        return self._generate_has_attr("name")

    def generate_has_jump(self) -> str:
        return self._generate_has_attr("jump")

    def generate_has_free(self) -> str:
        return self._generate_has_attr("free")

    def generate_has_local(self) -> str:
        return self._generate_has_attr("local")

    def generate_has_exc(self) -> str:
        return self._generate_has_attr("exc", flag_override="HAS_PURE_FLAG")

    def _generate_stack_effect(self, direction: str) -> str:
        """
        Adapted from https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Tools/cases_generator/stack.py#L89-L111
        """

        lines = []
        for ins in self:
            if ins.is_pseudo:
                continue

            stack = get_stack_effect(ins._inner)
            if direction == "popped":
                val = -stack.base_offset
            elif direction == "pushed":
                val = stack.top_offset - stack.base_offset

            expr = val.to_c()
            line = f"{ins.enum_variant} => {expr}"
            lines.append(line)

        conds = ",\n".join(lines)
        doc = "from" if direction == "popped" else "on"
        return f"""
/// How many items should be {direction} {doc} the stack.
pub const fn num_{direction}(&self, oparg: i32) -> i32 {{
    match &self {{
    {conds},
    _ => panic!("Pseudo opcodes are not allowed!")
    }}
}}
"""

    def generate_num_popped(self) -> str:
        return self._generate_stack_effect("popped")

    def generate_num_pushed(self) -> str:
        return self._generate_stack_effect("pushed")


def main():
    analysis = analyzer.analyze_files([DEFAULT_INPUT])
    instructions = Instructions(analysis)

    enum_variant_defs = instructions.generate_enum_variant_defs()
    is_valid = instructions.generate_is_valid()
    is_pseudo = instructions.generate_is_pseudo()

    has_arg = instructions.generate_has_arg()
    has_const = instructions.generate_has_const()
    has_name = instructions.generate_has_name()
    has_jump = instructions.generate_has_jump()
    has_free = instructions.generate_has_free()
    has_local = instructions.generate_has_local()
    has_exc = instructions.generate_has_exc()

    num_popped = instructions.generate_num_popped()
    num_pushed = instructions.generate_num_pushed()

    script_path = pathlib.Path(__file__).absolute().relative_to(ROOT).as_posix()

    opcode_src = f"""
/// Python opcode
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Opcode {{
{enum_variant_defs}
}}

impl Opcode {{
{is_valid}

{is_pseudo}

{has_arg}

{has_const}

{has_name}

{has_jump}

{has_free}

{has_local}

{has_exc}

{num_popped}

{num_pushed}
}}

macro_rules! opcode_try_from_impl {{
    ($t:ty) => {{
        impl TryFrom<$t> for Opcode {{
            type Error = ();

            fn try_from(value: $t) -> Result<Self, Self::Error> {{
                let id = value.try_into().map_err(|_| ())?;
                if Self::is_valid(id) {{
                    // SAFETY: We just validated that we have a valid opcode id.
                    Ok( unsafe {{ std::mem::transmute::<u16, Self>(id) }})
                }} else {{
                    Err(())
                }}
            }}
        }}
    }};
}}

opcode_try_from_impl!(i8);
opcode_try_from_impl!(i16);
opcode_try_from_impl!(i32);
opcode_try_from_impl!(i64);
opcode_try_from_impl!(i128);
opcode_try_from_impl!(isize);
opcode_try_from_impl!(u8);
opcode_try_from_impl!(u16);
opcode_try_from_impl!(u32);
opcode_try_from_impl!(u64);
opcode_try_from_impl!(u128);
opcode_try_from_impl!(usize);
    """.strip()

    out = f"""
//! Python opcode implementation. Currently aligned with cpython 3.13.7

// This file is generated by {script_path}
// Do not edit!

{opcode_src}
""".strip()

    OUT_PATH.write_text(out)
    print("DONE")

    print("Running `cargo fmt`")
    subprocess.run(["cargo", "fmt"], cwd=ROOT)


if __name__ == "__main__":
    main()
