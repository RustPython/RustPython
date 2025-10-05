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

ROOT = pathlib.Path(__file__).parents[1]
OUT_PATH = ROOT / "compiler" / "core" / "src" / "instruction.rs"


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
        return self._inner.name

    @property
    def has_oparg(self) -> bool:
        return self._inner.properties.oparg

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
    def associated_constant(self) -> str:
        return f"Self::{self.name}"

    @property
    def associated_constant_def(self) -> str:
        return f"pub const {self.name}: Self = unsafe {{ Self::new_unchecked({self.id}) }};"

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

    def generate_associated_constant_defs(self) -> str:
        return "\n".join(ins.associated_constant_def for ins in self)

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
        matches = " | ".join(ins.associated_constant for ins in self if ins.is_pseudo)
        return f"""
/// Whether opcode ID is pseudo.
#[must_use]
pub const fn is_pseudo(&self) -> bool {{
    matches!(self, {matches})
}}
""".strip()

    def _generate_has_attr(self, attr: str, *, flag_override: str | None = None) -> str:
        flag = flag_override if flag_override else f"has_{attr}_flag".upper()
        matches = " | ".join(
            ins.associated_constant for ins in self if flag in ins.flags
        )
        return f"""
/// Whether opcode ID have '{flag}' set.
#[must_use]
pub const fn has_{attr}(&self) -> bool {{
    matches!(self, {matches})
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


def main():
    analysis = analyzer.analyze_files([DEFAULT_INPUT])
    instructions = Instructions(analysis)

    associated_constant_defs = instructions.generate_associated_constant_defs()
    is_valid = instructions.generate_is_valid()
    is_pseudo = instructions.generate_is_pseudo()

    has_arg = instructions.generate_has_arg()
    has_const = instructions.generate_has_const()
    has_name = instructions.generate_has_name()
    has_jump = instructions.generate_has_jump()
    has_free = instructions.generate_has_free()
    has_local = instructions.generate_has_local()
    has_exc = instructions.generate_has_exc()

    script_path = pathlib.Path(__file__).absolute().relative_to(ROOT).as_posix()

    opcode_id_src = f"""
/// Represents a valid opcode ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpcodeId(u16);

impl OpcodeId {{
{associated_constant_defs}

    /// Creates a new Instruction without validating that the `id` is valid.
    #[must_use]
    pub const unsafe fn new_unchecked(id: u16) -> Self {{
        Self(id)
    }}

{is_valid}

{is_pseudo}

{has_arg}

{has_const}

{has_name}

{has_jump}

{has_free}

{has_local}

{has_exc}
}}

macro_rules! opcode_id_try_from_impl {{
    ($t:ty) => {{
        impl TryFrom<$t> for OpcodeId {{
            type Error = ();

            fn try_from(value: $t) -> Result<Self, Self::Error> {{
				let id = value.try_into().map_err(|_| ())?;
				if Self::is_valid(id) {{
					Ok(Self(id))
				}} else {{
					Err(())
				}}
			}}
		}}
    }};
}}

opcode_id_try_from_impl!(i8);
opcode_id_try_from_impl!(i16);
opcode_id_try_from_impl!(i32);
opcode_id_try_from_impl!(i64);
opcode_id_try_from_impl!(i128);
opcode_id_try_from_impl!(isize);
opcode_id_try_from_impl!(u8);
opcode_id_try_from_impl!(u16);
opcode_id_try_from_impl!(u32);
opcode_id_try_from_impl!(u64);
opcode_id_try_from_impl!(u128);
opcode_id_try_from_impl!(usize);
    """.strip()

    out = f"""
///! Python opcode implementation. Currently aligned with cpython 3.13.7

// This file is generated by {script_path}
// Do not edit!

{opcode_id_src}
""".strip()

    OUT_PATH.write_text(out)
    print("DONE")

    print("Running `cargo fmt`")
    subprocess.run(["cargo", "fmt"], cwd=ROOT)


if __name__ == "__main__":
    main()
