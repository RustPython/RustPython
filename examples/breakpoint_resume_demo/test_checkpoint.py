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
    parser = argparse.ArgumentParser(description="断点续运行功能测试")
    parser.add_argument(
        "--bin",
        help="rustpython 可执行文件路径，默认使用 target/release/rustpython",
    )
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    bin_path = resolve_bin_path(repo_root, args.bin)
    demo_path = Path(__file__).with_name("demo.py")
    snap_path = demo_path.with_suffix(".rpsnap")

    if not bin_path.exists():
        print(f"[error] rustpython 不存在: {bin_path}")
        return 1

    if snap_path.exists():
        snap_path.unlink()

    # 第一次运行：触发断点并生成快照
    result1 = run_cmd([str(bin_path), str(demo_path)])
    output1 = combined_output(result1)
    if result1.returncode != 0:
        print("[error] 第一次运行失败")
        print("stdout:")
        print(result1.stdout)
        print("stderr:")
        print(result1.stderr)
        return 1
    if output1.strip():
        if "phase=init" not in output1:
            print("[error] 第一次运行输出不符合预期")
            print("stdout:")
            print(result1.stdout)
            print("stderr:")
            print(result1.stderr)
            return 1
    if not snap_path.exists():
        print("[error] 断点文件未生成")
        return 1

    # 第二次运行：从断点 1 续跑到断点 2
    result2 = run_cmd([str(bin_path), "--resume", str(snap_path), str(demo_path)])
    output2 = combined_output(result2)
    if result2.returncode != 0:
        print("[error] 第二次运行失败")
        print("stdout:")
        print(result2.stdout)
        print("stderr:")
        print(result2.stderr)
        return 1
    if output2.strip():
        if "phase=after_checkpoint_1" not in output2:
            print("[error] 第二次运行输出不符合预期")
            print("stdout:")
            print(result2.stdout)
            print("stderr:")
            print(result2.stderr)
            return 1
    if not snap_path.exists():
        print("[error] 第二次运行后断点文件消失")
        return 1

    # 第三次运行：从断点 2 续跑并完成
    result3 = run_cmd([str(bin_path), "--resume", str(snap_path), str(demo_path)])
    output3 = combined_output(result3)
    if result3.returncode != 0:
        print("[error] 第三次运行失败")
        print("stdout:")
        print(result3.stdout)
        print("stderr:")
        print(result3.stderr)
        return 1
    if output3.strip():
        if "phase=after_checkpoint_2" not in output3 or "done" not in output3:
            print("[error] 第三次运行输出不符合预期")
            print("stdout:")
            print(result3.stdout)
            print("stderr:")
            print(result3.stderr)
            return 1
    if snap_path.exists():
        print("[error] 第三次运行后断点文件未清理")
        return 1

    print("[ok] 断点续运行测试通过")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
