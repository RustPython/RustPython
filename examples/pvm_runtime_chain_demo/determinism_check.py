import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

OUTPUT_RE = re.compile(r"output_hex=([0-9a-fA-F]+)")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def reset_state(root: Path) -> None:
    state_dir = root / "tmp" / "pvm_state"
    events_path = root / "tmp" / "pvm_events.log"
    if state_dir.exists():
        shutil.rmtree(state_dir)
    if events_path.exists():
        events_path.unlink()


def ensure_built(root: Path, env: dict) -> None:
    cmd = ["cargo", "build", "--release", "--example", "pvm_runtime_chain_demo"]
    subprocess.run(cmd, cwd=root, env=env, check=True)


def apply_dyld_fallback(env: dict) -> None:
    fallback = env.get("DYLD_FALLBACK_LIBRARY_PATH")
    if fallback:
        paths = fallback.split(os.pathsep)
    else:
        paths = []

    conda_prefix = env.get("CONDA_PREFIX")
    if conda_prefix:
        conda_lib = Path(conda_prefix) / "lib"
        if conda_lib.exists():
            paths.append(str(conda_lib))

    candidates = [
        Path("/opt/homebrew/opt/libffi/lib"),
        Path("/usr/local/opt/libffi/lib"),
        Path("/opt/homebrew/opt/libiconv/lib"),
        Path("/usr/local/opt/libiconv/lib"),
        Path("/opt/miniconda3/lib"),
    ]
    for candidate in candidates:
        if candidate.exists() and str(candidate) not in paths:
            paths.append(str(candidate))

    if paths:
        env["DYLD_FALLBACK_LIBRARY_PATH"] = os.pathsep.join(
            dict.fromkeys(paths)
        )


def run_once(root: Path, exe: Path, script: str, input_text: str, env: dict) -> str:
    cmd = [str(exe), script]
    if input_text:
        cmd.append(input_text)
    result = subprocess.run(
        cmd,
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise RuntimeError(f"execution failed with code {result.returncode}")
    text = result.stdout + result.stderr
    match = OUTPUT_RE.search(text)
    if not match:
        sys.stderr.write(text)
        raise RuntimeError("output_hex not found in output")
    return match.group(1)


def main() -> int:
    parser = argparse.ArgumentParser(description="Check determinism across runs.")
    parser.add_argument(
        "--script",
        default="examples/pvm_runtime_chain_demo/determinism_demo.py",
        help="Path to the Python contract (relative to repo root).",
    )
    parser.add_argument("--input", default="hello", help="Contract input string.")
    parser.add_argument("--runs", type=int, default=3, help="Number of runs to compare.")
    parser.add_argument(
        "--decode",
        action="store_true",
        help="Decode output hex as JSON for display.",
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument(
        "--reset-state",
        dest="reset_state",
        action="store_true",
        default=True,
        help="Reset tmp state between runs (default).",
    )
    group.add_argument(
        "--keep-state",
        dest="reset_state",
        action="store_false",
        help="Keep tmp state between runs.",
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip cargo build if the example binary already exists.",
    )
    args = parser.parse_args()

    root = repo_root()
    exe = root / "target" / "release" / "examples" / "pvm_runtime_chain_demo"
    env = os.environ.copy()
    env_build = env.copy()
    for key in ("DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES"):
        env_build.pop(key, None)

    if not args.skip_build or not exe.exists():
        ensure_built(root, env_build)

    env_run = env.copy()
    apply_dyld_fallback(env_run)

    outputs = []
    for idx in range(args.runs):
        if args.reset_state:
            reset_state(root)
        out_hex = run_once(root, exe, args.script, args.input, env_run)
        outputs.append(out_hex)
        print(f"run[{idx}] output_hex={out_hex}")
        if args.decode:
            try:
                decoded = bytes.fromhex(out_hex).decode("utf-8")
                parsed = json.loads(decoded)
                print(json.dumps(parsed, indent=2, sort_keys=True))
            except Exception as exc:
                print(f"decode failed: {exc}")

    first = outputs[0]
    mismatches = [idx for idx, value in enumerate(outputs) if value != first]
    if mismatches:
        print(f"determinism check failed: mismatched runs {mismatches}")
        return 1

    print(f"determinism check ok: {args.runs} runs match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
