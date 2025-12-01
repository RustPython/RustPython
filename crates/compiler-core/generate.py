#!/usr/bin/env python
from __future__ import annotations

import dataclasses
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


@dataclasses.dataclass(frozen=True, slots=True)
class OpargMetadata:
    name: str | None = None
    typ: str | None = None


@dataclasses.dataclass(slots=True)
class InstructionOverride:
    enabled: bool = True
    name: str | None = None
    oparg: OpargMetadata = dataclasses.field(default_factory=OpargMetadata)
    properties: analyzer.Properties | None = None

    def __post_init__(self):
        if isinstance(self.oparg, dict):
            self.oparg = OpargMetadata(**self.oparg)

        if isinstance(self.properties, dict):
            self.properties = dataclasses.replace(
                analyzer.SKIP_PROPERTIES, **self.properties
            )


@dataclasses.dataclass(slots=True)
class Instruction:
    # TODO: Maybe add a post_init hook to show warning incase of oparg being set for
    # instructions with no oparg?
    instruction: analyzer.Instruction | analyzer.PseudoInstruction
    override: InstructionOverride = dataclasses.field(
        default_factory=InstructionOverride
    )

    @property
    def rust_name(self) -> str:
        return self.override.name or snake_case_to_pascal_case(self.instruction.name)

    @property
    def rust_enum_variant(self) -> str:
        if self.properties.oparg:
            fields = f"{{ {self.oparg_name}: Arg<{self.oparg_typ}> }}"
        else:
            fields = ""

        return f"{self.rust_name} {fields} = {self.instruction.opcode}"

    @property
    def properties(self) -> analyzer.Properties:
        return self.override.properties or self.instruction.properties

    @property
    def oparg_name(self) -> str | None:
        if name := self.override.oparg.name:
            return name

        if not self.properties.oparg:
            return None

        oparg_names_map = build_oparg_names_map()
        if name := oparg_names_map.get(self.instruction.name):
            return name

        return self._oparg.field_name

    @property
    def oparg_typ(self) -> str | None:
        if typ := self.override.oparg.typ:
            return typ

        properties = self.properties
        if not properties.oparg:
            return None

        try:
            return self._oparg.name
        except ValueError:
            return "u32"  # Fallback

    @property
    def _oparg(self) -> Oparg:
        try:
            return Oparg.try_from_properties(self.properties)
        except ValueError as err:
            err.add_note(self.instruction.name)
            raise err

    @classmethod
    def from_analysis(
        cls, analysis: analyzer.Analysis, overrides: dict[str, dict]
    ) -> Iterator[typing.Self]:
        insts = {}
        for name, inst in analysis.instructions.items():
            override = InstructionOverride(**overrides.get(name, {}))
            if not override.enabled:
                continue

            opcode = inst.opcode
            insts[opcode] = cls(inst, override)

        # Because we are treating pseudos like real opcodes,
        # we need to find an alternative opcode for them (they go over u8::MAX)
        for opcode, inst in insts.items():
            if opcode <= U8_MAX:
                continue

            # Preserve `HAVE_ARG` semantics.
            if inst.properties.oparg:
                rang = range(analysis.have_arg, U8_MAX + 1)
            else:
                rang = range(0, analysis.have_arg)

            new_opcode = next(i for i in rang if i not in insts)
            inst.instruction.opcode = new_opcode

        yield from insts.values()


@enum.unique
class Oparg(enum.Enum):
    Label = enum.auto()
    NameIdx = enum.auto()

    @property
    def field_name(self) -> str:
        match self:
            case self.Label:
                return "target"
            case self.NameIdx:
                return "namei"

    @classmethod
    def try_from_properties(cls, properties: analyzer.Properties) -> typing.Self:
        # TODO: `properties.uses_co_consts` -> `ConstIdx`
        # TODO: `properties.uses_locals` -> `LocalIdx`

        if properties.uses_co_names:
            return cls.NameIdx
        elif properties.jumps:
            return cls.Label
        else:
            raise ValueError(f"Could not detect oparg type of {properties}")


@functools.cache
def build_oparg_names_map() -> dict[str, str]:
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


def write_enum(outfile: typing.IO, instructions: list[Instruction]) -> None:
    variants = ",\n".join(inst.rust_enum_variant for inst in instructions)
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
    overrides = conf["overrides"]

    instructions = sorted(
        Instruction.from_analysis(analysis, overrides), key=lambda inst: inst.rust_name
    )

    outfile = io.StringIO()
    write_enum(outfile, instructions)

    generated = outfile.getvalue()

    imports = ",".join(
        {
            inst.oparg_typ
            for inst in instructions
            if ((inst.oparg_typ is not None) and (inst.oparg_typ != "u32"))
        }
    )
    script_path = pathlib.Path(__file__).resolve().relative_to(ROOT).as_posix()
    output = rustfmt(
        f"""
    // This file is generated by {script_path}
    // Do not edit!

    use crate::bytecode::{{Arg, {imports}}};

    {generated}
    """
    )
    OUT_FILE.write_text(output)


if __name__ == "__main__":
    main()
