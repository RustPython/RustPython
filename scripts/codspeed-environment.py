#!/usr/bin/env python3
"""Name the machine a CodSpeed measurement was taken on, and hold a branch to it.

CodSpeed reports a run as a change against a base run, and the runner uploads a
description of the machine beside the profile: `SystemInfo` in
`CodSpeedHQ/runner` carries the OS and its version, the architecture, the CPU's
vendor id, brand and flags, the physical core count and the installed memory.
When the two runs disagree on it the report says "different runtime
environments detected" and the numbers below that line are not a comparison of
two trees.  A hosted runner pool hands out whatever machine is free, so two runs
of the same workflow do not share one by construction.

A simulated measurement does not move with how fast the machine is -- the cache
geometry the runner hands Valgrind is fixed, and two runs whose wall clock
differs by half still report the same counts.  It does move with what the
machine makes the process execute: glibc resolves `memcpy`, `memset` and the
string routines through IFUNC selectors keyed on CPU flags, so one binary has
one instruction stream per selection.  The toolchain is the same class of input
one level up, since a new stable rustc emits different code.  So is the CPython
these benchmarks are linked against: `benches/execution.rs` and
`benches/microbenchmarks.rs` run each program under CPython through pyo3 as well
as under RustPython, and the runner's default interpreter moves with its image.

The rule this enforces: `main` defines the environment.  Every run on it records
what it measured on, and a run on any other ref measures only when it matches
that recording.  A mismatch is skipped rather than uploaded, because an uploaded
cross-environment run is published as a regression of the branch that happened
to draw the other machine.

    codspeed-environment.py --record build/codspeed-environment.json
    codspeed-environment.py --record CURRENT --reference REFERENCE --github-output

Exits 0 whether or not the environments match; the verdict is the `match`
output, so the caller decides what a mismatch costs.
"""

import argparse
import hashlib
import json
import math
import os
import platform
import re
import subprocess
import sys

# The fields a comparison is made over.  `SystemInfo` also carries the host name
# and the invoking user, which a hosted runner changes on every run and which
# therefore cannot be part of a rule that any run is expected to satisfy.
COMPARED_FIELDS = (
    "os",
    "arch",
    "cpu_vendor_id",
    "cpu_brand",
    "cpu_cores",
    "total_memory_gb",
    "cpu_flags",
    "rustc",
    "libc",
    "python",
)


def _command_output(argv):
    try:
        out = subprocess.run(
            argv, capture_output=True, text=True, check=False, timeout=60
        )
    except (OSError, subprocess.SubprocessError):
        return ""
    # A probe that fails contributes nothing rather than an error string, so a
    # field absent on one platform stays absent instead of becoming a value.
    return out.stdout.strip() if out.returncode == 0 else ""


def _os_release():
    """The distribution and its version, as `SupportedOs` spells them."""
    try:
        with open("/etc/os-release", encoding="utf-8") as handle:
            fields = dict(
                line.rstrip("\n").split("=", 1)
                for line in handle
                if "=" in line and not line.startswith("#")
            )
    except OSError:
        return f"{platform.system()} {platform.release()}"
    name = fields.get("ID", "linux").strip('"')
    version = fields.get("VERSION_ID", "").strip('"')
    return f"{name} {version}".strip()


def _linux_cpu():
    with open("/proc/cpuinfo", encoding="utf-8") as handle:
        return parse_cpuinfo(handle.read())


def parse_cpuinfo(text):
    blocks = [block for block in text.split("\n\n") if block.strip()]

    def field(block, name):
        match = re.search(rf"^{name}\s*:\s*(.*)$", block, re.MULTILINE)
        return match.group(1).strip() if match else ""

    first = blocks[0] if blocks else ""
    # Count distinct physical cores the way `sysinfo::System::physical_core_count`
    # does: one entry per (socket, core), so a hyperthreaded pair counts once.
    # aarch64 names neither, and the file ends in a machine-wide block that no
    # `processor` line introduces, so the fallback counts those lines instead.
    cores = {(field(block, "physical id"), field(block, "core id")) for block in blocks}
    cores.discard(("", ""))
    logical = len(re.findall(r"^processor\s*:", text, re.MULTILINE))
    # x86_64 spells the CPU's identity one way and aarch64 another; a field
    # missing on one architecture is constant there and carries no signal.
    vendor = field(first, "vendor_id") or field(first, "CPU implementer")
    brand = (
        field(first, "model name")
        or " ".join(
            field(first, name) for name in ("CPU part", "CPU variant", "CPU revision")
        ).strip()
    )
    flags = field(first, "flags") or field(first, "Features")
    return {
        "cpu_vendor_id": vendor,
        "cpu_brand": brand,
        "cpu_cores": len(cores) or logical,
        "cpu_flags": sorted(flags.split()),
    }


def _darwin_cpu():
    def sysctl(name):
        return _command_output(["sysctl", "-n", name])

    flags = " ".join(
        sysctl(name) for name in ("machdep.cpu.features", "hw.optional.arm.caps")
    )
    return {
        "cpu_vendor_id": sysctl("machdep.cpu.vendor") or platform.machine(),
        "cpu_brand": sysctl("machdep.cpu.brand_string"),
        "cpu_cores": int(sysctl("hw.physicalcpu") or 0),
        "cpu_flags": sorted(flags.split()),
    }


def _total_memory_gb():
    """Installed memory in whole GiB, rounded up as `sysinfo` reports it."""
    try:
        with open("/proc/meminfo", encoding="utf-8") as handle:
            for line in handle:
                if line.startswith("MemTotal:"):
                    return math.ceil(int(line.split()[1]) * 1024 / 1024**3)
    except OSError:
        pass
    size = _command_output(["sysctl", "-n", "hw.memsize"])
    return math.ceil(int(size) / 1024**3) if size.isdigit() else 0


def _libc_version():
    """The C library whose IFUNC selectors decide the string routines."""
    version = _command_output(["ldd", "--version"])
    return version.splitlines()[0].strip() if version else ""


def _python_version():
    """The interpreter the benchmarks measure beside RustPython.

    Read from the process running this script rather than from a subprocess, so
    it names the `python3` the workflow put on PATH -- which is the one the
    pyo3 build resolves libpython from.
    """
    return f"{platform.python_implementation()} {platform.python_version()}"


def collect():
    cpu = _linux_cpu() if sys.platform.startswith("linux") else _darwin_cpu()
    environment = {
        "os": _os_release(),
        "arch": platform.machine(),
        "total_memory_gb": _total_memory_gb(),
        "rustc": _command_output(["rustc", "--version"]),
        "libc": _libc_version(),
        "python": _python_version(),
        **cpu,
    }
    environment["digest"] = digest(environment)
    return environment


def digest(environment):
    payload = json.dumps(
        {name: environment.get(name) for name in COMPARED_FIELDS},
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:16]


def differences(current, reference):
    return [
        (name, reference.get(name), current.get(name))
        for name in COMPARED_FIELDS
        if reference.get(name) != current.get(name)
    ]


def _render(environment):
    for name in COMPARED_FIELDS:
        value = environment.get(name)
        if name == "cpu_flags":
            value = f"{len(value)} flags"
        print(f"  {name:<17}{value}")
    print(f"  {'digest':<17}{environment['digest']}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--record", metavar="PATH", help="write this machine's environment as JSON"
    )
    parser.add_argument(
        "--reference",
        metavar="PATH",
        help="compare against a previously recorded environment; a missing file"
        " is not a mismatch, because there is nothing yet to disagree with",
    )
    parser.add_argument(
        "--github-output",
        action="store_true",
        help="append `match` and `digest` to $GITHUB_OUTPUT",
    )
    args = parser.parse_args(argv)

    current = collect()
    print("Benchmark environment:")
    _render(current)

    match = True
    if args.reference:
        try:
            with open(args.reference, encoding="utf-8") as handle:
                reference = json.load(handle)
        except (OSError, ValueError):
            print(
                f"\nNo reference environment at {args.reference}; recording this one."
            )
        else:
            moved = differences(current, reference)
            if moved:
                match = False
                # Digest the reference's own fields rather than reading back the
                # digest stored beside them, so a recording written by an older
                # field set is named by what it actually holds.
                print(
                    f"\nThis machine is not the one the reference was taken on"
                    f" ({digest(reference)} -> {current['digest']}):"
                )
                for name, was, now in moved:
                    if name == "cpu_flags":
                        was_set, now_set = set(was or ()), set(now or ())
                        was = " ".join(sorted(was_set - now_set)) or "-"
                        now = " ".join(sorted(now_set - was_set)) or "-"
                    print(f"  {name}: {was} -> {now}")
            else:
                print(f"\nThis machine matches the reference ({current['digest']}).")

    if args.record:
        directory = os.path.dirname(args.record)
        if directory:
            os.makedirs(directory, exist_ok=True)
        with open(args.record, "w", encoding="utf-8") as handle:
            json.dump(current, handle, indent=2, sort_keys=True)
            handle.write("\n")

    if args.github_output:
        if not match:
            # A skipped job is otherwise indistinguishable from one that never
            # had benchmarks, and the reason is the one fact a reader needs.
            print(
                "::notice title=CodSpeed benchmarks skipped::"
                f"This runner ({current['cpu_brand']}, {current['digest']}) is not"
                " the machine the comparison base was measured on, and an"
                " upload from it would be published as a regression."
            )
        output = os.environ.get("GITHUB_OUTPUT")
        if output:
            with open(output, "a", encoding="utf-8") as handle:
                handle.write(f"match={'true' if match else 'false'}\n")
                handle.write(f"digest={current['digest']}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
