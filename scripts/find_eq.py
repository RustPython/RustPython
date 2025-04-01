# Run differential queries to find equivalent files in cpython and rustpython
# Arguments
# --cpython: Path to cpython source code
# --print-diff: Print the diff between the files
# --color: Output color
# --files: Optional globbing pattern to match files in cpython source code 
# --checklist: output as checklist

import argparse
import difflib
import pathlib

parser = argparse.ArgumentParser(description="Find equivalent files in cpython and rustpython")
parser.add_argument("--cpython", type=pathlib.Path, required=True, help="Path to cpython source code")
parser.add_argument("--print-diff", action="store_true", help="Print the diff between the files")
parser.add_argument("--color", action="store_true", help="Output color")
parser.add_argument("--files", type=str, default="*.py", help="Optional globbing pattern to match files in cpython source code")

args = parser.parse_args()

if not args.cpython.exists():
    raise FileNotFoundError(f"Path {args.cpython} does not exist")
if not args.cpython.is_dir():
    raise NotADirectoryError(f"Path {args.cpython} is not a directory")
if not args.cpython.is_absolute():
    args.cpython = args.cpython.resolve()

cpython_lib = args.cpython / "Lib"
rustpython_lib = pathlib.Path(__file__).parent.parent / "Lib"
assert rustpython_lib.exists(), "RustPython lib directory does not exist, ensure the find_eq.py script is located in the right place"

# walk through the cpython lib directory
cpython_files = []
for path in cpython_lib.rglob(args.files):
    if path.is_file():
        # remove the cpython lib path from the file path
        path = path.relative_to(cpython_lib)
        cpython_files.append(path)

for path in cpython_files:
    # check if the file exists in the rustpython lib directory
    rustpython_path = rustpython_lib / path
    if rustpython_path.exists():
        # open both files and compare them
        try:
            with open(cpython_lib / path, "r") as cpython_file:
                cpython_code = cpython_file.read()
            with open(rustpython_lib / path, "r") as rustpython_file:
                rustpython_code = rustpython_file.read()
            # compare the files
            diff = difflib.unified_diff(cpython_code.splitlines(), rustpython_code.splitlines(), lineterm="", fromfile=str(path), tofile=str(path))
            # print the diff if there are differences
            diff = list(diff)
            if len(diff) > 0:
                if args.print_diff:
                    print("Differences:")
                    for line in diff:
                        print(line)
                else:
                    print(f"File is not identical: {path}")
            else:
                print(f"File is identical: {path}")
        except Exception as e:
            print(f"Unable to check file {path}: {e}")
    else:
        print(f"File not found in RustPython: {path}")

# check for files in rustpython lib directory that are not in cpython lib directory
rustpython_files = []
for path in rustpython_lib.rglob(args.files):
    if path.is_file():
        # remove the rustpython lib path from the file path
        path = path.relative_to(rustpython_lib)
        rustpython_files.append(path)
        if path not in cpython_files:
            print(f"File not found in CPython: {path}")
