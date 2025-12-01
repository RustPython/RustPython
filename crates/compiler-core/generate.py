#!/usr/bin/env python
from __future__ import annotations

import enum
import functools
import io
import pathlib
import subprocess
import sys
import typing

import tomllib

if typing.TYPE_CHECKING:
    from collections.abc import Iterator

CPYTHON_VERSION = "v3.13.9"


CRATE_ROOT = pathlib.Path(__file__).parent
CONF_FILE = CRATE_ROOT / "instructions.toml"
OUT_FILE = CRATE_ROOT / "src" / "bytecode" / "instruction.rs"

ROOT = CRATE_ROOT.parents[1]
SUBMODULES = ROOT / "submodules"
CPYTHON_DIR = SUBMODULES / f"cpython-{CPYTHON_VERSION}"
CPYTHON_TOOLS_DIR = CPYTHON_DIR / "Tools" / "cases_generator"
DIS_DOC = CPYTHON_DIR / "Doc" / "library" / "dis.rst"

sys.path.append(CPYTHON_TOOLS_DIR.as_posix())

import analyzer
from generators_common import DEFAULT_INPUT

U8_MAX = 255


class Inst:
    def __init__(
        self, cpython_name: str, override: dict, analysis: analyzer.Analysis
    ) -> None:
        inst = analysis.instructions[cpython_name]
        properties = inst.properties

        self.name = override.get("name", snake_case_to_pascal_case(cpython_name))
        self.id = analysis.opmap[cpython_name]
        self.oparg = override.get("oparg", properties.oparg)

        if (oparg_typ := override.get("oparg_typ")) is not None:
            self.oparg_typ = getattr(Oparg, oparg_typ)
        elif self.oparg:
            self.oparg_typ = Oparg.from_properties(properties)

        if (oparg_name := override.get("oparg_name")) is not None:
            self.oparg_name = oparg_name
        elif self.oparg:
            oparg_map = build_oparg_name_map()
            self.oparg_name = oparg_map.get(cpython_name, self.oparg_typ.field_name)

    @property
    def variant(self) -> str:
        if self.oparg:
            fields = f"{{ {self.oparg_name}: Arg<{self.oparg_typ.name}> }}"
        else:
            fields = ""

        return f"{self.name} {fields} = {self.id}"

    @classmethod
    def iter_insts(
        cls, analysis: analyzer.Analysis, conf: dict
    ) -> Iterator[typing.Self]:
        opcodes = conf["opcodes"]

        insts = {}
        for name in analysis.instructions:
            override = opcodes.get(name, {})
            if not override.get("enabled", True):
                continue

            inst = cls(name, override, analysis)
            insts[inst.id] = inst

        # Because we are treating pseudos like real opcodes,
        # we need to find an alternative ID for them (they go over u8::MAX)
        HAVE_ARG = analysis.have_arg
        occupied = set()
        for id_, inst in insts.items():
            if id_ < U8_MAX:
                continue

            if inst.oparg:
                ids = range(HAVE_ARG, U8_MAX + 1)
            else:
                ids = range(0, HAVE_ARG)

            new_id = next(i for i in ids if i not in occupied)
            occupied.add(new_id)
            inst.id = new_id

        yield from insts.values()

    def __lt__(self, other) -> bool:
        return self.name < other.name


@enum.unique
class Oparg(enum.StrEnum):
    IntrinsicFunction1 = enum.auto()
    IntrinsicFunction2 = enum.auto()
    ResumeKind = enum.auto()
    Label = enum.auto()
    NameIdx = enum.auto()
    u32 = enum.auto()  # TODO: Remove this; Everything needs to be a newtype

    @property
    def field_name(self) -> str:
        match self:
            case self.Label:
                return "target"
            case self.NameIdx:
                return "namei"
            case _:
                return "idx"  # Fallback to `idx`

    @classmethod
    def from_properties(cls, properties: analyzer.Properties) -> typing.Self:
        if properties.uses_co_names:
            return cls.NameIdx
        elif properties.jumps:
            return cls.Label
        elif properties.uses_co_consts:
            return cls.u32  # TODO: Needs to be `ConstIdx`
        elif properties.uses_locals:
            return cls.u32  # TODO: Needs to be `ConstIdx`
        else:
            # TODO: Raise here.
            return cls.u32  # Fallback to something generic


@functools.cache
def build_oparg_name_map() -> dict[str, str]:
    doc = DIS_DOC.read_text()

    out = {}
    for line in doc.splitlines():
        if not line.startswith(".. opcode:: "):
            continue

        # At this point `line` would look something like:
        #
        # `.. opcode:: OPCODE_NAME`
        # or
        # `.. opcode:: OPCODE_NAME (oparg_name)`
        #
        # We only care about the later.

        parts = line.split()
        if len(parts) != 4:
            continue

        _, _, cpython_name, oparg = parts
        out[cpython_name] = oparg.removeprefix("(").removesuffix(")")

    return out


def snake_case_to_pascal_case(name: str) -> str:
    return name.title().replace("_", "")


def rustfmt(code: str) -> str:
    return subprocess.check_output(["rustfmt", "--emit=stdout"], input=code, text=True)


def get_analysis() -> analyser.Analysis:
    analysis = analyzer.analyze_files([DEFAULT_INPUT])

    # We don't differentiate between real and pseudos yet
    analysis.instructions |= analysis.pseudos
    return analysis


def write_enum(outfile: typing.IO, instructions: list[Inst]) -> None:
    variants = ",\n".join(inst.variant for inst in instructions)
    outfile.write(
        f"""
    /// A Single bytecode instruction.
    #[repr(u8)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Instruction {{
        {variants}
    }}
    """
    )


def main():
    analysis = get_analysis()
    conf = tomllib.loads(CONF_FILE.read_text())
    instructions = sorted(Inst.iter_insts(analysis, conf))

    outfile = io.StringIO()

    write_enum(outfile, instructions)

    script_path = pathlib.Path(__file__).resolve().relative_to(ROOT).as_posix()

    generated = outfile.getvalue()
    output = rustfmt(
        f"""
    // This file is generated by {script_path}
    // Do not edit!

    use crate::bytecode::{{Arg, Label, NameIdx}};

    {generated}
    """
    )
    print(output)
    OUT_FILE.write_text(output)


if __name__ == "__main__":
    main()
