#!/usr/bin/env python
import collections
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
from stack import get_stack_effect


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class OpcodeGen:
    name: str
    instruction_enum: str
    instructions: list
    numeric_repr: str
    metadata: dict[str, str]
    analysis: analyzer.Analysis

    def gen(self) -> str:
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

        variants = ",\n".join(instr.name for instr in self)

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
        arms = ",\n".join(f"Self::{instr.name} => {instr.opcode}" for instr in self)
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
        arms = ",\n".join(f"{instr.opcode} => Self::{instr.name}" for instr in self)
        return f"""
        #[must_use]
        pub const fn from_{self.numeric_repr}(
            value: {self.numeric_repr}
        ) -> Result<Self, MarshalError> {{
            Ok(match value {{
                {arms},
                _ => return Err(MarshalError::InvalidBytecode),
            }})
        }}
        """

    @property
    def impl_tryfrom_numeric(self) -> str:
        return f"""
        impl TryFrom<{self.numeric_repr}> for {self.name} {{
            type Error = MarshalError;

            fn try_from(value: {self.numeric_repr}) -> Result<Self, Self::Error> {{
                Self::from_{self.numeric_repr}(value)
            }}
        }}
        """

    @property
    def impl_into_numeric(self) -> str:
        return f"""
        impl From<{self.name}> for {self.numeric_repr} {{
            fn from(opcode: {self.name}) -> Self {{
                opcode.as_{self.numeric_repr}()
            }}
        }}
        """

    def build_has_attr_fn(self, fn_attr: str, prop_attr: str, doc_flag: str) -> str:
        arms = "|".join(
            f"Self::{instr.name}"
            for instr in self
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

    @property
    def instrumented(self) -> list:
        return [instr for instr in self if instr.name.startswith("Instrumented")]

    @property
    def fn_to_base(self) -> str:
        inames = {instr.name for instr in self.instrumented}
        names = {instr.name for instr in self} - inames

        arms = ""
        for iname in sorted(inames):
            name = iname.removeprefix("Instrumented")
            if name not in names:
                continue
            arms += f"Self::{iname} => Self::{name},\n"

        arms = arms.strip()
        if not arms:
            return ""

        return f"""
        #[must_use]
        pub const fn to_base(self) -> Option<Self> {{
            Some(match self {{
                {arms}
                _ => return None,

            }})
        }}
        """

    @property
    def fn_to_instrumented(self) -> str:
        inames = {instr.name for instr in self.instrumented}
        names = {instr.name for instr in self} - inames

        arms = ""
        for iname in sorted(inames):
            name = iname.removeprefix("Instrumented")
            if name not in names:
                continue
            arms += f"Self::{name} => Self::{iname},\n"

        arms = arms.strip()
        if not arms:
            return ""

        return f"""
        #[must_use]
        pub const fn to_instrumented(self) -> Option<Self> {{
            Some(match self {{
                {arms}
                _ => return None,

            }})
        }}
        """

    @property
    def fn_deopt(self) -> str:
        names = {instr.name for instr in self}

        deopts = collections.defaultdict(list)
        for family in self.analysis.families.values():
            family_name = to_pascal_case(family.name)
            if family_name not in names:
                continue

            for member in family.members:
                if member.name == family_name:
                    continue

                deopts[family_name].append(member.name)

        arms = ""
        for target, specialized in deopts.items():
            ops = "|".join(f"Self::{op}" for op in specialized)
            arms += f"{ops} => Self::{target},\n"

        arms = arms.strip()
        if not arms:
            return ""

        return f"""
        #[must_use]
        pub const fn deopt(self) -> Option<Self> {{
            Some(match self {{
                {arms}
                _ => return None,
            }})
        }}
        """

    @property
    def fn_cache_entries(self) -> str:
        arms = ""
        for instr in self:
            name = instr.name
            if getattr(instr, "family", None) and (instr.family.name != name):
                continue

            if name.startswith("Instrumented"):
                continue

            try:
                size = instr.size
            except AttributeError:
                continue

            if size > 1:
                arms += f"Self::{name} => {size - 1},\n"

        arms = arms.strip()
        if not arms:
            return ""

        return f"""
        #[must_use]
        pub const fn cache_entries(self) -> usize {{
            match self {{
                {arms}
                _ => 0,
            }}
        }}
        """

    @property
    def fn_stack_effect(self) -> str:
        arms = ""
        for instr in self:
            stack = get_stack_effect(instr)
            popped = (-stack.base_offset).to_c()
            pushed = (stack.logical_sp - stack.base_offset).to_c()

            name = instr.name
            arms += f"Self::{name} => ({pushed}, {popped}),\n"

        arms = arms.strip()

        return f"""
        fn stack_effect_info(&self, oparg: u32) -> StackEffect {{
            // Reason for converting oparg to i32 is because of expressions like `1 + (oparg -1)`
            // that causes underflow errors.
            let oparg = i32::try_from(oparg).expect("oparg does not fit in an `i32`");

            let (pushed, popped) = match self {{
                {arms}
            }};

            debug_assert!(u32::try_from(pushed).is_ok());
            debug_assert!(u32::try_from(popped).is_ok());

            StackEffect::new(pushed as u32, popped as u32)
        }}
        """

    @property
    def fn_as_instruction(self) -> str:
        arms = ""
        for instr in self:
            name = instr.name
            arms += f"Self::{name} => {self.instruction_enum}::{name}"
            if oparg := self.metadata.get(name, {}).get("oparg"):
                oname = oparg["name"]
                arms += f" {{ {oname}: Arg::marker() }}"

            arms += ",\n"

        return f"""
        /// Returns self as [`{self.instruction_enum}`].
        #[must_use]
        pub const fn as_instruction(self) -> {self.instruction_enum} {{
            match self {{
                {arms}
            }}
        }}
        """

    @property
    def impl_as_instruction(self) -> str:
        return f"""
        impl From<{self.name}> for {self.instruction_enum} {{
            fn from(opcode: {self.name}) -> Self {{
                opcode.as_instruction()
            }}
        }}
        """

    def __iter__(self):
        yield from self.instructions


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class InstructioneGen:
    name: str
    opcode_enum: str
    instructions: list
    numeric_repr: str
    metadata: dict[str, str]

    def gen(self) -> str:
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

        variants = ""
        for instr in self:
            name = instr.name
            variants += name

            if oparg := self.metadata.get(name, {}).get("oparg"):
                oname, otype = oparg["name"], oparg["type"]

                variants += f"{{ {oname}: Arg<{otype}> }}"

            opcode = instr.opcode
            variants += f" = {opcode},\n"

        return f"""
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr({self.numeric_repr})] // TODO: Remove this `#[repr(...)]`
        pub enum {self.name} {{
            {variants}
        }}

        impl {self.name} {{
            {methods}
        }}

        {impls}
        """

    @property
    def fn_as_opcode(self) -> str:
        arms = ""
        for instr in self:
            name = instr.name
            arms += f"Self::{name}"
            if oparg := self.metadata.get(name, {}).get("oparg"):
                arms += " { .. }"

            arms += f"=> {self.opcode_enum}::{name},\n"

        return f"""
        /// Returns self as a [`{self.opcode_enum}`].
        #[must_use]
        pub const fn as_opcode(self) -> {self.opcode_enum} {{
            match self {{
                {arms}
            }}
        }}
        """

    @property
    def impl_as_opcode(self) -> str:
        return f"""
        impl From<{self.name}> for {self.opcode_enum} {{
            fn from(instruction: {self.name}) -> Self {{
                instruction.as_opcode()
            }}
        }}
        """

    def __iter__(self):
        yield from self.instructions


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

    outfile = io.StringIO()
    for opcode_enum, conf in CONF.items():
        metadata = conf["opcodes"]
        numeric_repr = conf["numeric_repr"]
        instruction_enum = conf["instruction_enum"]

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

        opcode_code = OpcodeGen(
            name=opcode_enum,
            instruction_enum=instruction_enum,
            instructions=instructions,
            numeric_repr=numeric_repr,
            metadata=metadata,
            analysis=analysis,
        ).gen()

        outfile.write(opcode_code)

        instruction_code = InstructioneGen(
            name=instruction_enum,
            opcode_enum=opcode_enum,
            instructions=instructions,
            numeric_repr=numeric_repr,
            metadata=metadata,
        ).gen()

        outfile.write(instruction_code)

    generated = outfile.getvalue()

    script_path = pathlib.Path(__file__).resolve().relative_to(ROOT).as_posix()

    output = rustfmt(
        f"""
// This file is generated by {script_path}
// Do not edit!

use crate::{{
    bytecode::{{instruction::StackEffect, oparg::Arg}},
    marshal::MarshalError,
}};

{generated}
    """
    )

    OUT_FILE.write_text(output)


if __name__ == "__main__":
    main()
