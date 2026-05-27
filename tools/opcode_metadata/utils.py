import dataclasses
import pathlib
import re
import sys

import tomllib

ROOT = pathlib.Path(__file__).parents[2].resolve()
DEFAULT_INPUT = ROOT / "crates/compiler-core/src/bytecode/instruction.rs"
DEFAULT_CONF = pathlib.Path(__file__).parent / "conf.toml"


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class StackEffect:
    pushed: str | None = None
    popped: str | None = None


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class Override:
    is_instrumented: bool | None = None
    stack_effect: StackEffect = dataclasses.field(default_factory=StackEffect)


type OverrideConfs = dict[str, Override]

SKIP_STACK_EFFECT = StackEffect()
SKIP_OVERRIDE = Override()


def get_conf(path: pathlib.Path = DEFAULT_CONF) -> OverrideConfs:
    data = path.read_text(encoding="utf-8")
    conf = tomllib.loads(data)
    for k, v in conf.items():
        v["stack_effect"] = StackEffect(**v.get("stack_effect", {}))
        conf[k] = Override(**v)

    return conf


def to_pascal_case(s: str) -> str:
    return s.title().replace("_", "")


def to_upper_snake_case(s: str) -> str:
    """
    Converts a PascalCaseString to be SNAKE_CASE

    Parameters
    ----------
    s : str
        Pascal cased string to convert.

    Returns
    -------
    str
        Uppercased snake case string.

    Examples
    --------
    >>> to_upper_snake_case("LoadAttr")
    LOAD_ATTR
    >>> to_upper_snake_case("CallIntrinsic1")
    CALL_INTRINSIC_1
    """
    res = re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", s)
    return re.sub(r"(\D)(\d+)$", r"\1_\2", res).upper()


def extract_enum_body(text: str, start: int) -> str:
    """
    Extract the rust enum body from a raw rust source code.

    Parameters
    ----------
    text : str
        Rust source code containing the enum body.
    start : int
        Offset to start searching from.

    Returns
    -------
    str
        Extracted enum body.
    """
    assert text[start] == "{"
    depth = 0
    for i, ch in enumerate(text[start:], start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return text[start + 1 : i].strip()  # exclude the outer braces

    raise ValueError("Could not find end to enum body")
