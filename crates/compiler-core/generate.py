#!/usr/bin/env python
import dataclasses
import io
import os
import pathlib
import subprocess
import sys

import tomllib

CRATE_ROOT = pathlib.Path(__file__).parent
CONF_FILE = CRATE_ROOT / "opcode.toml"
OUT_FILE = CRATE_ROOT / "src" / "bytecode" / "instructions.rs"

ROOT = CRATE_ROOT.parents[1]

try:
    CPYTHON_ROOT = pathlib.Path(os.environ["CPYTHON_ROOT"]).expanduser().resolve()
except KeyError:
    raise ValueError("Missing environment variable 'CPYTHON_ROOT'")

CPYTHON_TOOLS_LIB = CPYTHON_ROOT / "Tools" / "cases_generator"

sys.path.append(CPYTHON_TOOLS_LIB.as_posix())

import analyzer
from generators_common import DEFAULT_INPUT


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class OpcodeGen:
    name: str
    instructions: list
    numeric_repr: str

    def gen(self) -> str:
        variants = ",\n".join(instr.name for instr in self.instructions)

        methods = "\n\n".join(
            getattr(self, attr).strip()
            for attr in sorted(dir(self))
            if attr.startswith("fn_")
        )

        impls = "\n\n".join(
            getattr(self, attr).strip()
            for attr in sorted(dir(self))
            if attr.startswith("impl_")
        )

        return f"""
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum {self.name} {{
            {variants}
        }}

        impl {self.name} {{
            {methods}
        }}

        {impls}
        """

    @property
    def fn_as_numeric(self) -> str:
        arms = ",\n".join(
            f"Self::{instr.name} => {instr.opcode}" for instr in self.instructions
        )
        return f"""
        #[must_use]
        pub const fn as_{self.numeric_repr}(self) -> {self.numeric_repr} {{
            match self {{
                {arms},
            }}
        }}
        """

    @property
    def fn_tryfrom_numeric(self) -> str:
        arms = ",\n".join(
            f"{instr.opcode} => Self::{instr.name}" for instr in self.instructions
        )
        return f"""
        #[must_use]
        pub const fn from_{self.numeric_repr}(
            value: {self.numeric_repr}
        ) -> Result<Self, crate::marshal::MarshalError> {{
            Ok(match value {{
                {arms},
                _ => return Err(crate::MarshalError::InvalidBytecode),
            }})
        }}
        """

    @property
    def impl_tryfrom_numeric(self) -> str:
        return f"""
        impl TryFrom<{self.numeric_repr}> for {self.name} {{
            type Error = crate::marshal::MarshalError;

            fn try_from(value: {self.numeric_repr}) -> Result<Self, Self::Error> {{
                Self::from_{self.numeric_repr}(value)
            }}
        }}
        """

    @property
    def impl_into_numeric(self) -> str:
        return f"""
        impl From<{self.name}> for {self.numeric_repr}{{
            fn from(opcode: {self.name}) -> Self {{
                opcode.as_{self.numeric_repr}()
            }}
        }}
        """

    def build_has_attr_fn(self, fn_attr: str, prop_attr: str, doc_flag: str):
        arms = "|".join(
            f"Self::{instr.name}"
            for instr in self.instructions
            if getattr(instr.properties, prop_attr)
        )

        if arms:
            inner = f"matches!(self, {arms})"
        else:
            inner = "false"

        return f"""
        /// Does this opcode have '{doc_flag}' set.
        #[must_use]
        pub const fn has_{fn_attr}(self) -> bool {{
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


def to_pascal_case(s: str) -> str:
    return s.title().replace("_", "")


def get_analysis() -> analyzer.Analysis:
    analysis = analyzer.analyze_files([DEFAULT_INPUT])

    # We don't differentiate between real and pseudos yet
    analysis.instructions |= analysis.pseudos
    return analysis


def rustfmt(code: str) -> str:
    return subprocess.check_output(["rustfmt", "--emit=stdout"], input=code, text=True)


def main():
    CONF = tomllib.loads(CONF_FILE.read_text())

    analysis = get_analysis()

    opcodes_conf = CONF["Opcodes"]
    instructions_conf = CONF["Instructions"]

    outfile = io.StringIO()
    for key, conf in opcodes_conf.items():
        opcode_enum_name = conf["opcode_enum_name"]
        numeric_repr = conf["numeric_repr"]

        opcode_range = conf["range"]
        lower, upper = map(int, (opcode_range["min"], opcode_range["max"]))
        bounds = range(lower, upper + 1)

        instructions = sorted(
            (
                instr
                for instr in analysis.instructions.values()
                if instr.opcode in bounds
            ),
            key=lambda x: x.opcode,
        )

        for instr in instructions:
            instr.name = to_pascal_case(instr.name)

        code = OpcodeGen(
            name=opcode_enum_name, instructions=instructions, numeric_repr=numeric_repr
        ).gen()
        outfile.write(code)

    generated = outfile.getvalue()

    script_path = pathlib.Path(__file__).resolve().relative_to(ROOT).as_posix()

    output = rustfmt(
        f"""
// This file is generated by {script_path}
// Do not edit!

use crate::bytecode::Arg;

{generated}
    """
    )

    OUT_FILE.write_text(output)


if __name__ == "__main__":
    main()
