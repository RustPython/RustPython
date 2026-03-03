#!/usr/bin/env python
import collections
import dataclasses
import pathlib
import re
import subprocess
import urllib.request
from typing import TYPE_CHECKING

import tomllib

if TYPE_CHECKING:
    from collections.abc import Iterator

CPYTHON_VERSION = "3.14"

CONSTS_PATTERN = re.compile(r"\b_*[A-Z]+(?:_+[A-Z]+)*_*\b")

CARGO_TOML_FILE = pathlib.Path(__file__).parents[1] / "Cargo.toml"
CARGO_TOML = tomllib.loads(CARGO_TOML_FILE.read_text())
LIBC_DATA = CARGO_TOML["workspace"]["dependencies"]["libc"]


LIBC_VERSION = LIBC_DATA["version"] if isinstance(LIBC_DATA, dict) else LIBC_DATA
BASE_URL = f"https://raw.githubusercontent.com/rust-lang/libc/refs/tags/{LIBC_VERSION}"

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


def rustfmt(code: str) -> str:
    return subprocess.check_output(["rustfmt", "--emit=stdout"], input=code, text=True)


@dataclasses.dataclass(eq=True, frozen=True, slots=True)
class Cfg:
    inner: str

    def __str__(self) -> str:
        return self.inner

    def __lt__(self, other) -> bool:
        si, oi = map(str, (self.inner, other.inner))

        # Smaller length cfgs are smaller, regardless of value.
        return (len(si), si) < (len(oi), oi)


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class Target:
    cfgs: set[Cfg]
    sources: set[str] = dataclasses.field(default_factory=set)
    extras: set[str] = dataclasses.field(default_factory=set)


TARGETS = (
    Target(
        cfgs={Cfg('target_os = "android"')},
        sources={
            f"{BASE_URL}/src/unix/linux_like/android/mod.rs",
            f"{BASE_URL}/libc-test/semver/android.txt",
        },
    ),
    Target(
        cfgs={Cfg('target_os = "dragonfly"')},
        sources={
            f"{BASE_URL}/src/unix/bsd/freebsdlike/dragonfly/mod.rs",
            f"{BASE_URL}/libc-test/semver/dragonfly.txt",
        },
    ),
    Target(
        cfgs={Cfg('target_os = "freebsd"')},
        sources={
            f"{BASE_URL}/src/unix/bsd/freebsdlike/freebsd/mod.rs",
            f"{BASE_URL}/libc-test/semver/freebsd.txt",
        },
    ),
    Target(
        cfgs={Cfg('target_os = "linux"')},
        sources={
            f"{BASE_URL}/src/unix/linux_like/mod.rs",
            f"{BASE_URL}/src/unix/linux_like/linux_l4re_shared.rs",
            f"{BASE_URL}/libc-test/semver/linux.txt",
        },
    ),
    Target(
        cfgs={Cfg('target_os = "macos"')},
        sources={
            f"{BASE_URL}/src/unix/bsd/apple/mod.rs",
            f"{BASE_URL}/libc-test/semver/apple.txt",
        },
        extras={"COPYFILE_DATA as _COPYFILE_DATA"},
    ),
    Target(
        cfgs={Cfg('target_os = "netbsd"')},
        sources={
            f"{BASE_URL}/src/unix/bsd/netbsdlike/netbsd/mod.rs",
            f"{BASE_URL}/libc-test/semver/netbsd.txt",
        },
    ),
    Target(
        cfgs={Cfg('target_os = "redox"')},
        sources={
            f"{BASE_URL}/src/unix/redox/mod.rs",
            f"{BASE_URL}/libc-test/semver/redox.txt",
        },
    ),
    Target(cfgs={Cfg("unix")}, sources={f"{BASE_URL}/libc-test/semver/unix.txt"}),
)


def extract_consts(
    contents: str,
    *,
    pattern: re.Pattern = CONSTS_PATTERN,
    exclude: frozenset[str] = EXCLUDE,
) -> frozenset[str]:
    """
    Extract all words that are comprised from only uppercase letters + underscores.

    Parameters
    ----------
    contents : str
        Contents to extract the constants from.
    pattern : re.Pattern, Optional
        RE compiled pattern for extracting the consts.
    exclude : frozenset[str], Optional
        Items to exclude from the returned value.

    Returns
    -------
    frozenset[str]
        All constant names.
    """
    result = frozenset(pattern.findall(contents))
    return result - exclude


def consts_from_url(
    url: str, *, pattern: re.Pattern = CONSTS_PATTERN, exclude: frozenset[str] = EXCLUDE
) -> str:
    """
    Extract all consts from the contents found at the given URL.

    Parameters
    ----------
    url : str
        URL to fetch the contents from.
    pattern : re.Pattern, Optional
        RE compiled pattern for extracting the consts.
    exclude : frozenset[str], Optional
        Items to exclude from the returned value.

    Returns
    -------
    frozenset[str]
        All constant names at the URL.
    """
    try:
        with urllib.request.urlopen(url) as f:
            contents = f.read().decode()
    except urllib.error.HTTPError as err:
        err.add_note(url)
        raise

    return extract_consts(contents, pattern=pattern, exclude=exclude)


def main():
    # Step 1: Get all OS contants that we do want from upstream
    wanted_consts = consts_from_url(
        f"https://docs.python.org/{CPYTHON_VERSION}/library/os.html",
        # TODO: Exclude matches if they have `(` after (those are functions)
        pattern=re.compile(r"\bos\.(_*[A-Z]+(?:_+[A-Z]+)*_*)"),
    )

    # Step 2: build dict of what consts are available per cfg. `cfg -> {consts}`
    available = collections.defaultdict(set)
    for target in TARGETS:
        consts = set()
        for source in target.sources:
            consts |= consts_from_url(source)

        for cfg in target.cfgs:
            available[cfg] |= consts

    # Step 3: Keep only the "wanted" consts. Build a groupped mapping of `{cfgs} -> {consts}'
    groups = collections.defaultdict(set)
    available_items = available.items()
    for wanted_const in wanted_consts:
        cfgs = frozenset(
            cfg for cfg, consts in available_items if wanted_const in consts
        )
        if not cfgs:
            # We have no cfgs for a wanted const :/
            continue

        groups[cfgs].add(wanted_const)

    # Step 4: Build output
    output = ""
    for cfgs, consts in sorted(groups.items(), key=lambda t: (len(t[0]), sorted(t[0]))):
        target = next((target for target in TARGETS if target.cfgs == cfgs), None)
        if target:
            # If we found an exact target. Add its "extras" as-is
            consts |= target.extras

        cfgs_inner = ",".join(sorted(map(str, cfgs)))

        if len(cfgs) >= 2:
            cfgs_rust = f"#[cfg(any({cfgs_inner}))]"
        else:
            cfgs_rust = f"#[cfg({cfgs_inner})]"

        imports = ",".join(consts)
        entry = f"""
{cfgs_rust}
#[pyattr]
use libc::{{{imports}}};
""".strip()

        output += f"{entry}\n\n"

    print(rustfmt(output))


if __name__ == "__main__":
    main()
