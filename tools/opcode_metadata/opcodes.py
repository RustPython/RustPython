from __future__ import annotations

import collections
import dataclasses
import re
import typing
import warnings

import utils
from cpython import SKIP_PROPERTIES, Family, Properties, get_analysis, get_stack_effect
from utils import SKIP_OVERRIDE, Override, OverrideConfs, StackEffect, to_pascal_case

if typing.TYPE_CHECKING:
    from collections.abc import Iterable


@dataclasses.dataclass(frozen=True, slots=True)
class OpcodeInfo:
    enum_name: str
    size: str
    opcodes: tuple[Opcode, ...]

    @property
    def deopts(self) -> dict[str, list[str]]:
        analysis = get_analysis()
        names = {opcode.rust_name for opcode in self}

        res = collections.defaultdict(list)
        for family in analysis.families.values():
            family_name = to_pascal_case(family.name)
            if family_name not in names:
                continue

            for member in family.members:
                member_name = to_pascal_case(member.name)
                if member.name == family_name:
                    continue

                res[family_name].append(member_name)

        return dict(res)

    def __iter__(self):
        yield from self.opcodes

    @classmethod
    def iter_infos(
        cls, text: str, override_confs: OverrideConfs
    ) -> Iterable[typing.Self]:
        for block_match in re.finditer(
            r"define_opcodes!\s*\((.+?)\);", text, re.DOTALL
        ):
            block = block_match.group(1).strip()

            size = re.search(r"#\[repr\((\w+)\)\]", block).group(1)
            enum_name = re.search(
                r"#\[repr\(\w+\)\]\s*pub\s+enum\s+(\w+)\s*;", block
            ).group(1)

            second_enum_match = re.search(r"pub\s+enum\s+(\w+)\s*\{", block, re.DOTALL)
            entries = utils.extract_enum_body(block, second_enum_match.end() - 1)

            opcodes = tuple(sorted(iter_opcodes(entries, override_confs)))

            yield cls(enum_name, size, opcodes)


def iter_opcodes(text: str, override_confs: OverrideConfs) -> Iterable[Opcode]:
    analysis = get_analysis()
    # Split on commas that are followed by a newline + an uppercase letter (new entry)
    entries = map(str.strip, re.split(r",\s*\n\s*(?=[A-Z])", text))
    for entry in entries:
        if not entry:
            continue

        opcode = Opcode.from_str(entry)

        rust_name = opcode.rust_name
        override = override_confs.get(rust_name, SKIP_OVERRIDE)

        cpython_name = opcode.cpython_name

        kwargs = {}
        if instr := analysis.instructions.get(cpython_name):
            kwargs["properties"] = instr.properties
            kwargs["family"] = getattr(instr, "family", None)
            kwargs["cache_entry"] = getattr(instr, "size", -1)

            stack = get_stack_effect(instr)

            popped = (-stack.base_offset).to_c()
            pushed = (stack.logical_sp - stack.base_offset).to_c()
            kwargs["stack_effect"] = StackEffect(popped=popped, pushed=pushed)
        elif override == SKIP_OVERRIDE:
            warnings.warn(
                f"Could not get instruction metadata for {rust_name}"
                " from CPython or override conf"
            )

        yield dataclasses.replace(opcode, override=override, **kwargs)


@dataclasses.dataclass(frozen=True, slots=True)
class Opcode:
    rust_name: str
    id: int
    have_argument: bool = False
    cache_entry: int = 0
    stack_effect: StackEffect | None = None
    properties: Properties = dataclasses.field(default_factory=lambda: SKIP_PROPERTIES)
    family: Family | None = None
    override: Override = dataclasses.field(default_factory=Override)

    @property
    def is_instrumented(self) -> bool:
        if (res := self.override.is_instrumented) is not None:
            return res

        return self.cpython_name.startswith("INSTRUMENTED_")

    @property
    def cpython_name(self):
        return utils.to_upper_snake_case(self.rust_name)

    @property
    def cpy_popped(self) -> str | None:
        return getattr(self.stack_effect, "popped", None)

    @property
    def cpy_pushed(self) -> str | None:
        return getattr(self.stack_effect, "pushed", None)

    @property
    def stack_effect_popped(self) -> str:
        ove_popped = self.override.stack_effect.popped

        if (ove_popped is None) and (self.cpy_popped is None):
            raise ValueError(f"{self.rust_name} is missing popped stack_effect")

        return ove_popped or self.cpy_popped

    @property
    def stack_effect_pushed(self) -> str:
        ove_pushed = self.override.stack_effect.pushed

        if (ove_pushed is None) and (self.cpy_pushed is None):
            raise ValueError(f"{self.rust_name} is missing pushed stack_effect")

        return ove_pushed or self.cpy_pushed

    @classmethod
    def from_str(cls, entry: str) -> typing.Self:
        rust_name = re.match(r"(\w+)", entry).group(1)
        id_num = re.findall(r"= (\d+)", entry)[0]
        have_argument = "Arg<" in entry
        return cls(rust_name, int(id_num), have_argument=have_argument)

    def __lt__(self, other: typing.Self) -> bool:
        return self.id < other.id
