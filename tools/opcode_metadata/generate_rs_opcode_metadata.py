#!/usr/bin/env python
from __future__ import annotations

import collections
import dataclasses
import io
import os
import pathlib
import subprocess
import sys
import typing

import tomllib

from cpython import Analysis, get_analysis, get_stack_effect
from opcodes import OpcodeInfo
from utils import DEFAULT_INPUT, ROOT, get_conf, to_pascal_case

OUT_FILE = ROOT / "crates/compiler-core/src/bytecode/opcode_metadata.rs"


@dataclasses.dataclass(frozen=True, slots=True)
class OpcodeGen:
    info: OpcodeDef

    @property
    def fn_as_info_size(self) -> str:
        return f"""
        /// Returns [`Self`] as [`{self.size}`].
        #[must_use]
        #[inline]
        pub const fn as_{self.size}(self) -> {self.size} {{
            self.as_numeric()
        }}
        """

    @property
    def fn_try_from_numeric(self) -> str:
        return f"""
        pub const fn try_from_{self.size}(
            value: {self.size},
        ) -> Result<Self, MarshalError> {{
            Self::try_from_numeric(value)
        }}
        """

    @property
    def fn_has_arg(self) -> str:
        return self.gen_fn_has_attr("has_arg", "oparg", "HAS_ARG_FLAG")

    @property
    def fn_has_const(self) -> str:
        return self.gen_fn_has_attr("has_const", "uses_co_consts", "HAS_CONST_FLAG")

    @property
    def fn_has_name(self) -> str:
        return self.gen_fn_has_attr("has_name", "uses_co_names", "HAS_NAME_FLAG")

    @property
    def fn_has_jump(self) -> str:
        return self.gen_fn_has_attr("has_jump", "jumps", "HAS_JUMP_FLAG")

    @property
    def fn_has_free(self) -> str:
        return self.gen_fn_has_attr("has_free", "has_free", "HAS_FREE_FLAG")

    @property
    def fn_has_local(self) -> str:
        return self.gen_fn_has_attr("has_local", "uses_locals", "HAS_LOCAL_FLAG")

    @property
    def fn_has_eval_break(self) -> str:
        return self.gen_fn_has_attr(
            "has_eval_break", "eval_breaker", "HAS_EVAL_BREAK_FLAG"
        )

    @property
    def fn_is_instrumented(self) -> str:
        arms = "|".join(
            f"Self::{opcode.rust_name}" for opcode in self if opcode.is_instrumented
        )

        arms = arms.strip()
        if arms:
            inner = f"matches!(self, {arms})"
        else:
            inner = "false"

        return f"""
        #[must_use]
        pub const fn is_instrumented(self) -> bool {{
            {inner}
        }}
        """

    @property
    def fn_to_base(self) -> str:
        arms = ",\n".join(
            f"Self::{iname} => Self::{name}"
            for name, iname in self.instrumented_mapping.items()
        )

        arms = arms.strip()
        if not arms:
            inner = "None"
        else:
            inner = f"""
            Some(match self {{
                {arms},
                _ => return None,

            }})
            """

        return f"""
        #[must_use]
        #[inline]
        pub const fn to_base(self) -> Option<Self> {{
            {inner}
        }}
        """

    @property
    def fn_to_instrumented(self) -> str:
        arms = ",\n".join(
            f"Self::{name} => Self::{iname}"
            for name, iname in self.instrumented_mapping.items()
        )

        arms = arms.strip()
        if not arms:
            inner = "None"
        else:
            inner = f"""
            Some(match self {{
                {arms},
                _ => return None,

            }})
            """

        return f"""
        #[must_use]
        pub const fn to_instrumented(self) -> Option<Self> {{
            {inner}
        }}
        """

    @property
    def fn_deopt(self) -> str:
        specialized_to_base = self.specialized_to_base

        if not specialized_to_base:
            inner = "None"
        else:
            table_type = f"super::{self.info.enum_name}"
            entries = ",\n".join(
                f"Some({table_type}::{specialized_to_base[name]})"
                if name in specialized_to_base
                else "None"
                for name in self.rust_names_by_id
            )

            inner = f"""
            const DEOPT: [Option<{table_type}>; {self.table_size}] = [
                {entries}
            ];

            DEOPT[self.as_numeric() as usize]
            """

        return f"""
        #[must_use]
        #[inline]
        pub const fn deopt(self) -> Option<Self> {{
            {inner}
        }}
        """

    @property
    def fn_cache_entries(self) -> str:
        entries_by_base: dict[str, int] = {}
        for opcode in self:
            name = opcode.rust_name
            if opcode.is_instrumented:
                continue
            if getattr(opcode, "family", None) and (opcode.family.name != name):
                continue

            try:
                size = opcode.cache_entry
            except AttributeError:
                continue

            if size > 1:
                entries_by_base[name] = size - 1

        if not entries_by_base:
            inner = "0"
        else:
            entries = ", ".join(
                str(entries_by_base.get(self.resolve_deoptimized(name), 0))
                for name in self.rust_names_by_id
            )

            inner = f"""
            const CACHE_ENTRIES: [u8; {self.table_size}] = [{entries}];

            CACHE_ENTRIES[self.as_numeric() as usize] as usize
            """

        return f"""
        #[must_use]
        #[inline]
        pub const fn cache_entries(self) -> usize {{
            {inner}
        }}
        """

    @property
    def fn_stack_effect_info(self) -> str:
        oparg_used = False
        arms = ""
        for opcode in self:
            name = opcode.rust_name

            popped = opcode.stack_effect_popped
            pushed = opcode.stack_effect_pushed

            pushed_comment = ""
            popped_comment = ""

            if popped != opcode.cpy_popped:
                popped_comment = f"// TODO: Differs from CPython `{opcode.cpy_popped}`"

            if pushed != opcode.cpy_pushed:
                pushed_comment = f"// TODO: Differs from CPython `{opcode.cpy_pushed}`"

            oparg_used = oparg_used or any("oparg" in expr for expr in (pushed, popped))

            arms += f"""
                Self::{name} => (
                    {pushed}, {pushed_comment}
                    {popped}, {popped_comment}
                ),
            """.strip()

        arms = arms.strip()

        oparg_arg = "_oparg"
        oparg_cast = ""
        if oparg_used:
            oparg_arg = "oparg"
            oparg_cast = f"""
            // Reason for converting {oparg_arg} to i32 is because of expressions like `1 + (oparg -1)`
            // that causes underflow errors.
            let oparg = i32::try_from({oparg_arg}).expect("{oparg_arg} does not fit in an `i32`");
            """

        return f"""
        #[must_use]
        pub fn stack_effect_info(&self, {oparg_arg}: u32) -> StackEffect {{
            {oparg_cast}

            let (pushed, popped) = match self {{
                {arms}
            }};

            debug_assert!(u32::try_from(pushed).is_ok());
            debug_assert!(u32::try_from(popped).is_ok());

            StackEffect::new(pushed as u32, popped as u32)
        }}
        """

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

        return f"""
        impl super::{self.info.enum_name} {{
            {methods}
        }}

        {impls}
        """

    def gen_fn_has_attr(self, fn_name: str, properties_attr: str, doc_flag: str) -> str:
        arms = "|".join(
            f"Self::{opcode.rust_name}"
            for opcode in self
            if getattr(opcode.properties, properties_attr)
        )

        if arms:
            inner = f"matches!(self, {arms})"
        else:
            inner = "false"

        return f"""
        /// Does this opcode have '{doc_flag}' set.
        #[must_use]
        pub const fn {fn_name}(self) -> bool {{
            {inner}
        }}
        """

    @property
    def instrumented_mapping(self) -> dict[str, str]:
        names, inames = set(), set()
        for opcode in self:
            name = opcode.rust_name
            if opcode.is_instrumented:
                inames.add(name)
            else:
                names.add(name)

        res = {}
        for iname in sorted(inames):
            name = iname.removeprefix("Instrumented")
            if name not in names:
                continue

            res[name] = iname

        return res

    @property
    def specialized_to_base(self) -> dict[str, str]:
        """Maps a specialized opcode's name to its family's base opcode name."""
        res = {}
        for target, specialized in self.info.deopts.items():
            for name in specialized:
                res[name] = target

        return res

    @property
    def instrumented_to_base(self) -> dict[str, str]:
        """Maps an instrumented opcode's name to its base opcode name."""
        return {iname: name for name, iname in self.instrumented_mapping.items()}

    def resolve_deoptimized(self, name: str) -> str:
        """
        Mirrors `deoptimize`: resolves a specialized opcode to its family's
        base, an instrumented opcode to its base, or returns the name
        unchanged.
        """
        if name in self.specialized_to_base:
            return self.specialized_to_base[name]

        return self.instrumented_to_base.get(name, name)

    @property
    def table_size(self) -> int:
        return {"u8": 256, "u16": 65536}[self.size]

    @property
    def rust_names_by_id(self) -> list[str | None]:
        """The opcode name at each numeric id, `None` where no opcode is assigned."""
        names_by_id = {opcode.id: opcode.rust_name for opcode in self}
        return [names_by_id.get(i) for i in range(self.table_size)]

    @property
    def size(self) -> str:
        return self.info.size

    def __iter__(self):
        yield from self.info.opcodes


def rustfmt(code: str) -> str:
    return subprocess.check_output(["rustfmt", "--emit=stdout"], input=code, text=True)


def main():
    override_conf = get_conf()
    inp = DEFAULT_INPUT.read_text()
    opcode_infos = OpcodeInfo.iter_infos(inp, override_conf)

    outfile = io.StringIO()

    for info in opcode_infos:
        gen = OpcodeGen(info).gen()
        outfile.write(gen)

    generated = outfile.getvalue()

    script_path = pathlib.Path(__file__).resolve().relative_to(ROOT).as_posix()

    output = rustfmt(
        f"""
// This file is generated by {script_path}
// Do not edit!

use crate::{{
    bytecode::instruction::StackEffect,
    marshal::MarshalError,
}};

{generated}
    """
    )

    OUT_FILE.write_text(output)


if __name__ == "__main__":
    main()
