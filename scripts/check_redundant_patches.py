#!/usr/bin/env python
import ast
import pathlib
import sys

ROOT = pathlib.Path(__file__).parents[1]
TEST_DIR = ROOT / "Lib" / "test"


def main():
    exit_status = 0
    for file in TEST_DIR.rglob("**/*.py"):
        try:
            contents = file.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue

        try:
            tree = ast.parse(contents)
        except SyntaxError:
            continue

        for node in ast.walk(tree):
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
                rel = file.relative_to(ROOT)
                lineno = node.lineno
                print(
                    f"{rel}:{name}:{lineno} is a test patch that can be safely removed",
                    file=sys.stderr,
                )
    return exit_status


if __name__ == "__main__":
    exit(main())
