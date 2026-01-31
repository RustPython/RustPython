#!/usr/bin/env python
import collections
import re
import urllib.request
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from collections.abc import Iterator

CONSTS_PAT = re.compile(r"\b_*[A-Z]+(?:_+[A-Z]+)*_*\b")
OS_CONSTS_PAT = re.compile(
    r"\bos\.(_*[A-Z]+(?:_+[A-Z]+)*_*)"
)  # TODO: Exclude matches if they have `(` after (those are functions)


LIBC_VERSION = "0.2.180"

EXCLUDE = frozenset(
    {
        # Defined at `vm/src/stdlib/os.rs`
        "O_APPEND",
        "O_CREAT",
        "O_EXCL",
        "O_RDONLY",
        "O_RDWR",
        "O_TRUNC",
        "O_WRONLY",
        "SEEK_CUR",
        "SEEK_END",
        "SEEK_SET",
        # Functions, not consts
        "WCOREDUMP",
        "WIFCONTINUED",
        "WIFSTOPPED",
        "WIFSIGNALED",
        "WIFEXITED",
        "WEXITSTATUS",
        "WSTOPSIG",
        "WTERMSIG",
        # False positive
        # "EOF",
    }
)

EXTRAS = {
    frozenset({"macos"}): {"COPYFILE_DATA"},
}
RENAMES = {"COPYFILE_DATA": "_COPYFILE_DATA"}


def build_url(fname: str) -> str:
    return f"https://raw.githubusercontent.com/rust-lang/libc/refs/tags/{LIBC_VERSION}/libc-test/semver/{fname}.txt"


TARGET_OS = {
    "android": "android",
    "dragonfly": "dragonfly",
    "freebsd": "freebsd",
    "linux": "linux",
    "macos": "apple",
    "netbsd": "netbsd",
    "openbsd": "openbsd",
    "redox": "redox",
    # solaris?
    "unix": "unix",
}


def get_consts(url: str, pattern: re.Pattern = CONSTS_PAT) -> frozenset[str]:
    with urllib.request.urlopen(url) as f:
        resp = f.read().decode()

    return frozenset(pattern.findall(resp)) - EXCLUDE


def format_groups(groups: dict) -> "Iterator[tuple[str, str]]":
    # sort by length, then alphabet. so we will have a consistent output
    for targets, consts in sorted(
        groups.items(), key=lambda t: (len(t[0]), sorted(t[0]))
    ):
        cond = ", ".join(
            f'target_os = "{target_os}"' if target_os != "unix" else target_os
            for target_os in sorted(targets)
        )
        if len(targets) > 1:
            cond = f"any({cond})"
        cfg = f"#[cfg({cond})]"

        imports = ", ".join(
            const if const not in RENAMES else f"{const} as {RENAMES[const]}"
            for const in sorted(consts)
        )
        use = f"use libc::{{{imports}}};"
        yield cfg, use


def main():
    wanted_consts = get_consts(
        "https://docs.python.org/3.14/library/os.html",  # Should we read from https://github.com/python/cpython/blob/bcee1c322115c581da27600f2ae55e5439c027eb/Modules/posixmodule.c#L17023 instead?
        pattern=OS_CONSTS_PAT,
    )
    available = {
        target_os: get_consts(build_url(fname))
        for target_os, fname in TARGET_OS.items()
    }

    group_consts = collections.defaultdict(set)
    for const in wanted_consts:
        target_oses = frozenset(
            target_os for target_os, consts in available.items() if const in consts
        )
        if not target_oses:
            continue

        group_consts[target_oses].add(const)
    group_consts = {grp: v | EXTRAS.get(grp, set()) for grp, v in group_consts.items()}

    code = "\n\n".join(
        f"""
{cfg}
#[pyattr]
{use}
""".strip()
        for cfg, use in format_groups(group_consts)
    )

    print(code)


if __name__ == "__main__":
    main()
