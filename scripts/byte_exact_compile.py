"""Strict finite compiler comparison. This does not establish general parity."""

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def requests(fixtures, selected):
    names = [fixture["name"] for fixture in fixtures]
    if len(names) != len(set(names)):
        raise ValueError("duplicate fixture names")
    unknown = set(selected) - set(names)
    if unknown:
        raise ValueError("unknown fixtures: " + ", ".join(sorted(unknown)))
    chosen = [f for f in fixtures if not selected or f["name"] in selected]
    if not chosen:
        raise ValueError("empty requested corpus")
    result = []
    for fixture in chosen:
        if not fixture["name"].isascii() or not fixture["source"].isascii():
            raise ValueError("only fixed ASCII fixtures are supported")
        for optimize in (0, 1, 2):
            result.append(
                {
                    **fixture,
                    "filename": "byte_exact/" + fixture["name"] + ".py",
                    "optimize": optimize,
                }
            )
    return result


def differences(left, right, path=""):
    if type(left) is not type(right):
        return [path]
    if isinstance(left, dict):
        if left.keys() != right.keys():
            return [path]
        return [
            p
            for key in left
            for p in differences(left[key], right[key], path + "/" + key)
        ]
    if isinstance(left, list):
        if len(left) != len(right):
            return [path]
        return [
            p
            for i, (a, b) in enumerate(zip(left, right))
            for p in differences(a, b, path + "/" + str(i))
        ]
    return [] if left == right else [path]


def validate_cache_prefix(path):
    path = Path(path).resolve()
    if not path.is_dir() or any(path.iterdir()):
        raise ValueError("CPython cache prefix must be an existing empty directory")
    return path


def compare_captures(reference, candidate):
    return {
        key: differences(reference[key], candidate[key], key)
        for key in ("marshal", "magic", "code")
    }


def capture_environment(lib, cpython_cache_prefix):
    env = {
        "PATH": os.environ.get("PATH", ""),
        "LC_ALL": "C.UTF-8",
        "TZ": "UTC",
        "PYTHONHASHSEED": "0",
        "PYTHONDONTWRITEBYTECODE": "1",
        # CPython redirects normal source-cache reads before startup imports.
        # RustPython currently ignores this setting; its embedded Lib path must
        # independently be verified cache-free.
        "PYTHONPYCACHEPREFIX": str(cpython_cache_prefix),
    }
    if lib:
        env["RUSTPYTHONPATH"] = str(Path(lib).resolve())
    return env


def run_capture(executable, request, lib, cpython_cache_prefix):
    env = capture_environment(lib, cpython_cache_prefix)
    proc = subprocess.run(
        [executable, "-S", "-B", str(ROOT / "byte_exact_capture.py")],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        env=env,
        timeout=60,
    )
    if proc.returncode:
        raise RuntimeError(f"capture exit {proc.returncode}: {proc.stderr}")
    return json.loads(proc.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--candidate-lib", required=True)
    parser.add_argument("--cpython-cache-prefix", type=Path, required=True)
    parser.add_argument(
        "--fixtures", type=Path, default=ROOT / "byte_exact_fixtures.json"
    )
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        cache_prefix = validate_cache_prefix(args.cpython_cache_prefix)
        corpus = requests(json.loads(args.fixtures.read_text()), args.case)
    except ValueError as error:
        parser.error(str(error))
    rows = []
    report = {
        "status": "incomplete",
        "expected": len(corpus),
        "rows": rows,
        "reference_sha256": digest(args.reference),
        "candidate_sha256": digest(args.candidate),
        "harness_sha256": digest(__file__),
        "capture_sha256": digest(ROOT / "byte_exact_capture.py"),
        "fixtures_sha256": digest(args.fixtures),
        "cpython_pycache_prefix": str(cache_prefix),
        "candidate_lib": str(Path(args.candidate_lib).resolve()),
    }

    def save():
        args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    save()
    for request in corpus:
        row = {"request": request, "status": "incomplete"}
        rows.append(row)
        try:
            reference = run_capture(args.reference, request, None, cache_prefix)
            candidate = run_capture(
                args.candidate, request, args.candidate_lib, cache_prefix
            )
            validate_cache_prefix(cache_prefix)
            mismatches = compare_captures(reference, candidate)
            row.update(
                reference=reference,
                candidate=candidate,
                mismatches=mismatches,
                status="exact" if not any(mismatches.values()) else "mismatch",
            )
        except Exception as error:
            row.update(status="error", error=str(error))
            save()
            raise
        save()
    report.update(status="complete", exact=sum(r["status"] == "exact" for r in rows))
    save()
    print(json.dumps({"completed": len(rows), "exact": report["exact"]}))
    return 0 if report["exact"] == len(corpus) else 1


if __name__ == "__main__":
    sys.exit(main())
