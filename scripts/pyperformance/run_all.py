#!/usr/bin/env python3
"""Run the upstream pyperformance benchmark suite (https://github.com/python/pyperformance)
against a Python executable -- RustPython, a real CPython, or both -- and
catalog which benchmarks pass, fail, or time out under each.

Background
----------
pyperformance cannot be installed as-is on RustPython: its runtime dependency
`pyperf` hard-depends on `psutil`, a C-extension package. RustPython has no
CPython C-API / extension-module loading support (no `_imp.create_dynamic` /
`_imp.exec_dynamic`, no compiler config vars such as LDCXXSHARED in
`_sysconfigdata`), so building or loading any C extension fails.

Workaround: `pyperf` itself already disables psutil usage on interpreters
that report `Py_GIL_DISABLED=1` (see `pyperf._utils.USE_PSUTIL`), which is
exactly what RustPython reports (it has no GIL). So a functionality-free,
pure-Python "psutil" stub (see stub_psutil/) is enough to satisfy pip's
dependency resolution -- pyperf never actually calls into it at runtime on
RustPython. This unblocks any *pure-Python* pyperformance benchmark; any
benchmark whose own workload (not just pyperf) requires a real C extension
(e.g. lxml, numpy, greenlet) will still fail, and that failure is a genuine
finding: it marks a real C-extension gap in RustPython, not a tooling
artifact of this script. A real CPython target doesn't need this workaround
at all -- pass --no-psutil-stub for it, so it installs and uses the genuine
psutil.

This script:
  1. Ensures a *host* CPython venv with pyperformance installed (pyperformance
     itself only runs under a real CPython; the --python target is just the
     interpreter it benchmarks, which may be that same CPython or RustPython).
  2. Builds the pure-Python psutil stub wheel once (skipped with --no-psutil-stub).
  3. Runs every benchmark pyperformance knows about, one at a time, against
     the given --python target, with a timeout per benchmark.
  4. Writes a JSON + Markdown catalog of the results under results/<label>/.

Usage
-----
    python3 scripts/pyperformance/run_all.py \\
        --python target/release/rustpython --label rustpython

    python3 scripts/pyperformance/run_all.py \\
        --python "$(command -v python3.13)" --label cpython3.13 --no-psutil-stub

Then compare two labels' catalogs with compare.py (see its docstring).

Re-running resumes from results/<label>/catalog.json by default (skips
benchmarks already recorded); pass --force to redo everything.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
STUB_PSUTIL_DIR = SCRIPT_DIR / "stub_psutil"

DEFAULT_CACHE_DIR = REPO_ROOT / "target" / "pyperformance"
DEFAULT_OUT_DIR = SCRIPT_DIR / "results"
DEFAULT_TIMEOUT = 180


def log(msg: str) -> None:
    print(f"[pyperformance-runner] {msg}", flush=True)


def find_host_python() -> str:
    for candidate in (
        "python3.13",
        "python3.12",
        "python3.11",
        "python3.10",
        "python3",
    ):
        path = shutil.which(candidate)
        if path:
            return path
    raise SystemExit("No usable host CPython found on PATH (need python3.10+)")


def ensure_host_venv(cache_dir: Path) -> Path:
    venv_dir = cache_dir / "host-venv"
    pip_marker = venv_dir / "pyvenv.cfg"
    pyperf_installed = False
    if pip_marker.exists():
        pip_bin = venv_dir / "bin" / "pip"
        result = subprocess.run(
            [str(pip_bin), "show", "pyperformance"],
            capture_output=True,
            text=True,
        )
        pyperf_installed = result.returncode == 0
    if not pyperf_installed:
        log(
            f"setting up host venv at {venv_dir} (this runs pyperformance's CLI; "
            "RustPython is only the --python target it benchmarks)"
        )
        shutil.rmtree(venv_dir, ignore_errors=True)
        venv_dir.parent.mkdir(parents=True, exist_ok=True)
        host_python = find_host_python()
        subprocess.run([host_python, "-m", "venv", str(venv_dir)], check=True)
        pip_bin = venv_dir / "bin" / "pip"
        subprocess.run([str(pip_bin), "install", "-q", "-U", "pip"], check=True)
        subprocess.run([str(pip_bin), "install", "-q", "pyperformance"], check=True)
    return venv_dir


def ensure_stub_psutil_wheel(host_venv: Path, cache_dir: Path) -> Path:
    stub_pkgs_dir = cache_dir / "stub-pkgs"
    existing = list(stub_pkgs_dir.glob("psutil-*.whl"))
    if existing:
        return stub_pkgs_dir
    log(
        "building pure-Python psutil stub wheel (see scripts/pyperformance/stub_psutil/)"
    )
    stub_pkgs_dir.mkdir(parents=True, exist_ok=True)
    pip_bin = host_venv / "bin" / "pip"
    subprocess.run(
        [
            str(pip_bin),
            "wheel",
            str(STUB_PSUTIL_DIR),
            "-w",
            str(stub_pkgs_dir),
            "--no-deps",
            "-q",
        ],
        check=True,
    )
    return stub_pkgs_dir


def list_benchmarks(host_venv: Path) -> list[str]:
    pyperformance_bin = host_venv / "bin" / "pyperformance"
    result = subprocess.run(
        [str(pyperformance_bin), "list"],
        capture_output=True,
        text=True,
        check=True,
    )
    names = []
    for line in result.stdout.splitlines():
        line = line.strip()
        if line.startswith("- "):
            names.append(line[2:].strip())
    return names


MEAN_RE = re.compile(r"Mean \+- std dev:\s*(.+)")
FAILED_BUILD_RE = re.compile(r"Failed to build (\S+)")
FAILED_WHEEL_RE = re.compile(r"Failed building wheel for (\S+)")
LDCXXSHARED_RE = re.compile(
    r"Unexpected None in config vars: \[('LDCXXSHARED'[^\]]*)\]"
)
# The root-cause exception raised by the *benchmark script itself* (running under
# RustPython), as opposed to pyperformance's own wrapper exceptions
# (e.g. "RuntimeError: Benchmark died", "RuntimeError: ... failed with exit code ...")
# which are always the LAST exception in the combined output, not the first.
PYPERFORMANCE_WRAPPER_ERRORS = {
    "Benchmark died",
}
EXCEPTION_LINE_RE = re.compile(
    r"^(?:\w+\.)*(\w*(?:Error|Exception)): (.+)$", re.MULTILINE
)


def classify_failure(combined_output: str) -> str:
    if LDCXXSHARED_RE.search(combined_output):
        pkg_match = FAILED_WHEEL_RE.search(combined_output) or FAILED_BUILD_RE.search(
            combined_output
        )
        pkg = pkg_match.group(1) if pkg_match else "unknown package"
        return f"C-extension build blocked (no compiler config / no CPython C-API) building {pkg}"
    pkg_match = FAILED_WHEEL_RE.search(combined_output) or FAILED_BUILD_RE.search(
        combined_output
    )
    if pkg_match:
        return f"failed to build/install {pkg_match.group(1)}"
    # Prefer the first real exception in the log: it's raised by the benchmark
    # script running under RustPython, before pyperformance's own wrapper
    # exceptions (always last) obscure it with a generic "Benchmark died".
    for exc_type, exc_msg in EXCEPTION_LINE_RE.findall(combined_output):
        if exc_msg.strip() in PYPERFORMANCE_WRAPPER_ERRORS:
            continue
        if "failed with exit code" in exc_msg:
            continue
        return f"{exc_type}: {exc_msg.strip()}"[:300]
    m = re.search(r"^(ERROR: .*)$", combined_output, re.MULTILINE)
    if m:
        return m.group(1)[:200]
    tail = "\n".join(combined_output.strip().splitlines()[-5:])
    return tail[:400] or "unknown failure"


def run_one_benchmark(
    pyperformance_bin: Path,
    target_python: Path,
    bench: str,
    work_dir: Path,
    out_dir: Path,
    stub_pkgs_dir: Path | None,
    timeout: int,
    extra_args: list[str],
) -> dict:
    result_json = out_dir / "raw" / f"{bench}.json"
    result_json.parent.mkdir(parents=True, exist_ok=True)
    if result_json.exists():
        result_json.unlink()

    env = os.environ.copy()
    cmd = [
        str(pyperformance_bin),
        "run",
        "--python",
        str(target_python),
        "-b",
        bench,
        "-o",
        str(result_json),
        *extra_args,
    ]
    inherited = []
    if stub_pkgs_dir is not None:
        # Work around the target interpreter's lack of a C-extension psutil (see
        # module docstring). Real CPython doesn't need this -- pass
        # stub_pkgs_dir=None for it so it installs and uses the real psutil.
        env["PIP_FIND_LINKS"] = str(stub_pkgs_dir)
        inherited.append("PIP_FIND_LINKS")
    if "RUSTPYTHONPATH" in env:
        # A RustPython binary copied away from its checkout (as CI does when it
        # keeps one build of each commit around) can only find the stdlib
        # through this variable, and pyperf re-executes the target interpreter
        # in a venv of its own, so it has to survive that hop too.
        inherited.append("RUSTPYTHONPATH")
    if inherited:
        # One comma-separated flag, not one flag per variable: pyperf's
        # `--inherit-environ` is a plain (non-appending) option, so repeating it
        # keeps only the last name.
        cmd += ["--inherit-environ", ",".join(inherited)]

    log(f"running {bench} ...")
    try:
        proc = subprocess.run(
            cmd,
            cwd=work_dir,
            env=env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as exc:

        def _to_str(x):
            if isinstance(x, bytes):
                return x.decode("utf-8", "replace")
            return x or ""

        combined = _to_str(exc.stdout) + "\n" + _to_str(exc.stderr)
        return {
            "benchmark": bench,
            "status": "timeout",
            "detail": f"exceeded {timeout}s timeout",
            "mean": None,
        }

    combined = (proc.stdout or "") + "\n" + (proc.stderr or "")
    if proc.returncode == 0 and result_json.exists():
        mean_match = MEAN_RE.search(combined)
        return {
            "benchmark": bench,
            "status": "ok",
            "detail": None,
            "mean": mean_match.group(1).strip() if mean_match else None,
        }

    return {
        "benchmark": bench,
        "status": "fail",
        "detail": classify_failure(combined),
        "mean": None,
    }


def write_catalog(results: list[dict], out_dir: Path, label: str) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    catalog_json = out_dir / "catalog.json"
    catalog_json.write_text(json.dumps(results, indent=2, sort_keys=False) + "\n")

    ok = [r for r in results if r["status"] == "ok"]
    fail = [r for r in results if r["status"] == "fail"]
    timeout = [r for r in results if r["status"] == "timeout"]

    lines = []
    lines.append(f"# pyperformance on {label} -- benchmark catalog")
    lines.append("")
    lines.append(
        f"Generated by `scripts/pyperformance/run_all.py --label {label}`. "
        f"`{label}` was benchmarked as the `--python` target of upstream "
        "`pyperformance`. See the script docstring for the psutil-stub workaround "
        "used for interpreters without CPython C-API / C-extension support."
    )
    lines.append("")
    lines.append(
        f"**{len(ok)} passed, {len(fail)} failed, {len(timeout)} timed out** "
        f"out of {len(results)} benchmarks."
    )
    lines.append("")
    lines.append(f"| Benchmark | Status | Mean ({label}) | Notes |")
    lines.append("|---|---|---|---|")
    for r in sorted(results, key=lambda r: (r["status"] != "ok", r["benchmark"])):
        status_label = {"ok": "✅ pass", "fail": "❌ fail", "timeout": "⏱ timeout"}[
            r["status"]
        ]
        mean = r["mean"] or ""
        detail = (r["detail"] or "").replace("|", "\\|").replace("\n", " ")
        lines.append(f"| {r['benchmark']} | {status_label} | {mean} | {detail} |")
    lines.append("")

    catalog_md = out_dir / "CATALOG.md"
    catalog_md.write_text("\n".join(lines) + "\n")
    log(f"wrote {catalog_json} and {catalog_md}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--python",
        "--rustpython",
        dest="python",
        type=Path,
        default=REPO_ROOT / "target" / "release" / "rustpython",
        help="path to the Python (or RustPython) executable to benchmark "
        "(default: target/release/rustpython)",
    )
    parser.add_argument(
        "--label",
        type=str,
        default=None,
        help="name for this target, used for the results subdirectory and report "
        "headings (default: the executable's basename, e.g. 'rustpython' or "
        "'python3.13')",
    )
    parser.add_argument(
        "--no-psutil-stub",
        action="store_true",
        help="don't inject the pure-Python psutil stub -- use this for a real "
        "CPython target, which can install and use the genuine psutil",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=DEFAULT_TIMEOUT,
        help="per-benchmark timeout in seconds (default: %(default)s)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=DEFAULT_OUT_DIR,
        help="parent directory to write <label>/catalog.json and <label>/CATALOG.md into",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=DEFAULT_CACHE_DIR,
        help="directory for the host venv and stub wheel cache (default: target/pyperformance)",
    )
    parser.add_argument(
        "--benchmarks",
        type=str,
        default=None,
        help="comma-separated list of benchmarks to run (default: everything pyperformance knows)",
    )
    parser.add_argument(
        "--fast",
        action="store_true",
        default=True,
        help="pass --fast to pyperformance run (default: on, for a quicker catalog run)",
    )
    parser.add_argument(
        "--rigorous",
        action="store_true",
        help="pass --rigorous to pyperformance run instead of --fast",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="re-run benchmarks even if already present in an existing catalog.json",
    )
    args = parser.parse_args()

    target_python = shutil.which(str(args.python)) or str(args.python)
    target_python = Path(target_python)
    if not target_python.exists():
        raise SystemExit(
            f"Python executable not found: {args.python} "
            "(for RustPython, build it first, e.g. `cargo build --release --features ssl`)"
        )
    target_python = target_python.resolve()
    label = args.label or target_python.name

    out_dir = args.out / label

    extra_args = ["--rigorous"] if args.rigorous else ["--fast"]

    host_venv = ensure_host_venv(args.cache_dir)
    stub_pkgs_dir = (
        None
        if args.no_psutil_stub
        else ensure_stub_psutil_wheel(host_venv, args.cache_dir)
    )
    pyperformance_bin = host_venv / "bin" / "pyperformance"

    benchmarks = (
        [b.strip() for b in args.benchmarks.split(",") if b.strip()]
        if args.benchmarks
        else list_benchmarks(host_venv)
    )
    log(f"{len(benchmarks)} benchmarks to run against {label} ({target_python})")

    work_dir = args.cache_dir / "work" / label
    work_dir.mkdir(parents=True, exist_ok=True)

    # `all_results` starts as *every* benchmark previously recorded (not just the
    # ones in this invocation's --benchmarks subset), so a partial/targeted re-run
    # never drops earlier results from catalog.json/CATALOG.md -- it only updates
    # the entries it actually re-ran.
    all_results: dict[str, dict] = {}
    catalog_json = out_dir / "catalog.json"
    if catalog_json.exists():
        for r in json.loads(catalog_json.read_text()):
            all_results[r["benchmark"]] = r

    for bench in benchmarks:
        if bench in all_results and not args.force:
            log(f"skipping {bench} (already in catalog; use --force to redo)")
            continue
        r = run_one_benchmark(
            pyperformance_bin,
            target_python,
            bench,
            work_dir,
            out_dir,
            stub_pkgs_dir,
            args.timeout,
            extra_args,
        )
        log(
            f"  -> {bench}: {r['status']}"
            + (f" ({r['detail']})" if r["detail"] else "")
        )
        all_results[bench] = r
        write_catalog(
            list(all_results.values()), out_dir, label
        )  # incremental, so a crash keeps progress

    write_catalog(list(all_results.values()), out_dir, label)
    ran = [all_results[b] for b in benchmarks]
    ok = sum(1 for r in ran if r["status"] == "ok")
    log(
        f"done: {ok}/{len(ran)} requested benchmarks passed "
        f"({len(all_results)} total in catalog for {label})"
    )


if __name__ == "__main__":
    main()
