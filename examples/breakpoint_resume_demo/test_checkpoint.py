from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


def resolve_bin_path(root: Path, bin_override: str | None) -> Path:
    if bin_override:
        return Path(bin_override)

    bin_path = root / "target" / "release" / "pvm"
    if os.name == "nt":
        bin_path = bin_path.with_suffix(".exe")
    return bin_path


def run_cmd(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=False)


def combined_output(result: subprocess.CompletedProcess[str]) -> str:
    return (result.stdout or "") + (result.stderr or "")


def main() -> int:
    parser = argparse.ArgumentParser(description="Checkpoint resume functionality test")
    parser.add_argument(
        "--bin",
        help="Path to the rustpython executable, defaults to target/release/rustpython",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    bin_path = resolve_bin_path(repo_root, args.bin)
    demo_path = Path(__file__).with_name("demo.py")
    snap_path = demo_path.with_suffix(".rpsnap")

    if not bin_path.exists():
        print(f"[error] rustpython not found: {bin_path}")
        return 1

    if snap_path.exists():
        snap_path.unlink()

    # First run: hit checkpoint and generate snapshot
    result1 = run_cmd([str(bin_path), str(demo_path)])
    output1 = combined_output(result1)
    if result1.returncode != 0:
        print("[error] First run failed")
        print("stdout:")
        print(result1.stdout)
        print("stderr:")
        print(result1.stderr)
        return 1
    if output1.strip():
        if "phase=init" not in output1:
            print("[error] First run output did not match expectation")
            print("stdout:")
            print(result1.stdout)
            print("stderr:")
            print(result1.stderr)
            return 1
    if not snap_path.exists():
        print("[error] Snapshot file not generated")
        return 1

    # Second run: resume from checkpoint 1 to checkpoint 2
    result2 = run_cmd([str(bin_path), "--resume", str(snap_path), str(demo_path)])
    output2 = combined_output(result2)
    if result2.returncode != 0:
        print("[error] Second run failed")
        print("stdout:")
        print(result2.stdout)
        print("stderr:")
        print(result2.stderr)
        return 1
    if output2.strip():
        if "phase=after_checkpoint_1" not in output2:
            print("[error] Second run output did not match expectation")
            print("stdout:")
            print(result2.stdout)
            print("stderr:")
            print(result2.stderr)
            return 1
    if not snap_path.exists():
        print("[error] Snapshot file missing after second run")
        return 1

    # Third run: resume from checkpoint 2 and complete
    result3 = run_cmd([str(bin_path), "--resume", str(snap_path), str(demo_path)])
    output3 = combined_output(result3)
    if result3.returncode != 0:
        print("[error] Third run failed")
        print("stdout:")
        print(result3.stdout)
        print("stderr:")
        print(result3.stderr)
        return 1
    if output3.strip():
        if "phase=after_checkpoint_2" not in output3 or "done" not in output3:
            print("[error] Third run output did not match expectation")
            print("stdout:")
            print(result3.stdout)
            print("stderr:")
            print(result3.stderr)
            return 1
    if snap_path.exists():
        print("[error] Snapshot file not cleaned up after third run")
        return 1

    print("[ok] Checkpoint resume test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
