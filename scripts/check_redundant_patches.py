#!/usr/bin/env python
import argparse
import ast
import glob
import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).parents[1]
TEST_DIR = ROOT / "Lib" / "test"

IS_GH_CI = "GITHUB_ACTIONS" in os.environ


def build_argparser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="check_redundant_patches")
    parser.add_argument(
        "patterns",
        nargs="*",
        default=[f"{TEST_DIR}/**/*.py"],
        help="Glob patterns (e.g. foo/bar/**.py a/b/file.py)",
    )

    return parser


def iter_files(patterns: list[str]):
    seen = set()
    for pattern in set(patterns):
        matches = glob.glob(pattern, recursive=True)
        for path in matches:
            if path in seen:
                continue
            seen.add(path)
            yield path


def main(patterns: list[str]):
    exit_status = 0
    for file in map(pathlib.Path, iter_files(patterns)):
        if file.is_dir():
            continue

        try:
            contents = file.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue

        try:
            tree = ast.parse(contents)
        except SyntaxError:
            continue

        cls_name = None
        for node in ast.walk(tree):
            if isinstance(node, ast.ClassDef):
                cls_name = node.name

            if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue

            name = node.name
            if not name.startswith("test"):
                continue

            if node.decorator_list:
                continue

            func_code = ast.unparse(node.body)
            if func_code in (
                f"await super().{name}()",
                f"return await super().{name}()",
                f"return super().{name}()",
                f"super().{name}()",
            ):
                exit_status += 1

                lineno = node.lineno
                msg = f"{file}:{lineno}:{cls_name}.{name} is a test patch that can be safely removed"
                if IS_GH_CI:
                    end_lineno = node.end_lineno
                    msg = f"::error file={file},line={lineno},endLine={end_lineno},title=Redundant Test Patch::{msg}"

                print(msg, file=sys.stderr)

    return exit_status


if __name__ == "__main__":
    parser = build_argparser()
    args = parser.parse_args()
    exit(main(args.patterns))
