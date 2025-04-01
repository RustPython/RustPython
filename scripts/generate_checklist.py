# Arguments
# --cpython: Path to cpython source code
# --updated-libs: Libraries that have been updated in RustPython


import argparse
import dataclasses
import difflib
import pathlib
from typing import Optional
import warnings

import requests
from jinja2 import Environment, FileSystemLoader

parser = argparse.ArgumentParser(description="Find equivalent files in cpython and rustpython")
parser.add_argument("--cpython", type=pathlib.Path, required=True, help="Path to cpython source code")
parser.add_argument("--updated-libs", type=pathlib.Path, required=False,
                    help="Libraries that have been updated in RustPython")
parser.add_argument("--notes", type=pathlib.Path, required=False, help="Path to notes file")

args = parser.parse_args()

def check_pr(pr_id) -> bool:
    if pr_id.startswith("#"):
        pr_id = pr_id[1:]
    pr_id = int(pr_id)
    req = f"https://api.github.com/repos/RustPython/RustPython/pulls/{pr_id}"
    response = requests.get(req).json()
    return response["merged_at"] is not None

@dataclasses.dataclass
class LibUpdate:
    pr: Optional[str] = None
    done: bool = True

updated_libs = {}
if args.updated_libs:
    # check if the file exists in the rustpython lib directory
    updated_libs_path = args.updated_libs
    if updated_libs_path.exists():
        with open(updated_libs_path) as f:
            for line in f:
                line = line.strip()
                if not line.startswith("//") and line:
                    line = line.split(" ")
                    if len(line) == 2:
                        is_done = True
                        try:
                            is_done = check_pr(line[1])
                        except Exception as e:
                            warnings.warn(f"Failed to check PR {line[1]}: {e}")
                        updated_libs[line[0]] = LibUpdate(line[1])
                    elif len(line) == 1:
                        updated_libs[line[0]] = LibUpdate()
                    else:
                        raise ValueError(f"Invalid line: {line}")

    else:
        raise FileNotFoundError(f"Path {updated_libs_path} does not exist")
if not args.cpython.exists():
    raise FileNotFoundError(f"Path {args.cpython} does not exist")
if not args.cpython.is_dir():
    raise NotADirectoryError(f"Path {args.cpython} is not a directory")
if not args.cpython.is_absolute():
    args.cpython = args.cpython.resolve()

notes = {}
if args.notes:
    # check if the file exists in the rustpython lib directory
    notes_path = args.notes
    if notes_path.exists():
        with open(notes_path) as f:
            for line in f:
                line = line.strip()
                if not line.startswith("//") and line:
                    line = line.split(" ")
                    if len(line) > 1:
                        rest = " ".join(line[1:])
                        if line[0] in notes:
                            notes[line[0]].append(rest)
                        else:
                            notes[line[0]] = [rest]
                    else:
                        raise ValueError(f"Invalid note: {line}")

    else:
        raise FileNotFoundError(f"Path {notes_path} does not exist")

cpython_lib = args.cpython / "Lib"
rustpython_lib = pathlib.Path(__file__).parent.parent / "Lib"
assert rustpython_lib.exists(), "RustPython lib directory does not exist, ensure the find_eq.py script is located in the right place"

ignored_objs = [
    "__pycache__",
    "test"
]
# loop through the top-level directories in the cpython lib directory
libs = []
for path in cpython_lib.iterdir():
    if path.is_dir() and path.name not in ignored_objs:
        # add the directory name to the list of libraries
        libs.append(path.name)
    elif path.is_file() and path.name.endswith(".py") and path.name not in ignored_objs:
        # add the file name to the list of libraries
        libs.append(path.name)

tests = []
cpython_lib_test = cpython_lib / "test"
for path in cpython_lib_test.iterdir():
    if path.is_dir() and path.name not in ignored_objs and path.name.startswith("test_"):
        # add the directory name to the list of libraries
        tests.append(path.name)
    elif path.is_file() and path.name.endswith(".py") and path.name not in ignored_objs and path.name.startswith("test_"):
        # add the file name to the list of libraries
        file_name = path.name.replace("test_", "")
        if file_name not in libs and file_name.replace(".py", "") not in libs:
            tests.append(path.name)

def check_diff(file1, file2):
    try:
        with open(file1, "r") as f1, open(file2, "r") as f2:
            f1_lines = f1.readlines()
            f2_lines = f2.readlines()
            diff = difflib.unified_diff(f1_lines, f2_lines, lineterm="")
            diff_lines = list(diff)
            return len(diff_lines)
    except UnicodeDecodeError:
        return False

def check_completion_pr(display_name):
    for lib in updated_libs:
        if lib == str(display_name):
            return updated_libs[lib].done, updated_libs[lib].pr
    return False, None

def check_test_completion(rustpython_path, cpython_path):
    if rustpython_path.exists() and rustpython_path.is_file():
        if cpython_path.exists() and cpython_path.is_file():
            if not rustpython_path.exists() or not rustpython_path.is_file():
                return False
            elif check_diff(rustpython_path, cpython_path) > 0:
                return False
            return True
    return False

def check_lib_completion(rustpython_path, cpython_path):
    test_name = "test_" + rustpython_path.name
    rustpython_test_path = rustpython_lib / "test" / test_name
    cpython_test_path = cpython_lib / "test" / test_name
    if cpython_test_path.exists() and not check_test_completion(rustpython_test_path, cpython_test_path):
        return False
    if rustpython_path.exists() and rustpython_path.is_file():
        if check_diff(rustpython_path, cpython_path) > 0:
            return False
        return True
    return False

def handle_notes(display_path) -> list[str]:
    if str(display_path) in notes:
        res = notes[str(display_path)]
        # remove the note from the notes list
        del notes[str(display_path)]
        return res
    return []

@dataclasses.dataclass
class Output:
    name: str
    pr: Optional[str]
    completed: Optional[bool]
    notes: list[str]

update_libs_output = []
add_libs_output = []
for path in libs:
    # check if the file exists in the rustpython lib directory
    rustpython_path = rustpython_lib / path
    # remove the file extension if it exists
    display_path = pathlib.Path(path).with_suffix("")
    (completed, pr) = check_completion_pr(display_path)
    if rustpython_path.exists():
        if not completed:
            # check if the file exists in the cpython lib directory
            cpython_path = cpython_lib / path
            # check if the file exists in the rustpython lib directory
            if rustpython_path.exists() and rustpython_path.is_file():
                completed = check_lib_completion(rustpython_path, cpython_path)
        update_libs_output.append(Output(str(display_path), pr, completed, handle_notes(display_path)))
    else:
        if pr is not None and completed:
            update_libs_output.append(Output(str(display_path), pr, None, handle_notes(display_path)))
        else:
            add_libs_output.append(Output(str(display_path), pr, None, handle_notes(display_path)))

update_tests_output = []
add_tests_output = []
for path in tests:
    # check if the file exists in the rustpython lib directory
    rustpython_path = rustpython_lib / "test" / path
    # remove the file extension if it exists
    display_path = pathlib.Path(path).with_suffix("")
    (completed, pr) = check_completion_pr(display_path)
    if rustpython_path.exists():
        if not completed:
            # check if the file exists in the cpython lib directory
            cpython_path = cpython_lib / "test" / path
            # check if the file exists in the rustpython lib directory
            if rustpython_path.exists() and rustpython_path.is_file():
                completed = check_lib_completion(rustpython_path, cpython_path)
        update_tests_output.append(Output(str(display_path), pr, completed, handle_notes(display_path)))
    else:
        if pr is not None and completed:
            update_tests_output.append(Output(str(display_path), pr, None, handle_notes(display_path)))
        else:
            add_tests_output.append(Output(str(display_path), pr, None, handle_notes(display_path)))

for note in notes:
    # add a warning for each note that is not attached to a file
    for n in notes[note]:
        warnings.warn(f"Unattached Note: {note} - {n}")

env = Environment(loader=FileSystemLoader('.'))
template = env.get_template("checklist_template.md")
output = template.render(
    update_libs=update_libs_output,
    add_libs=add_libs_output,
    update_tests=update_tests_output,
    add_tests=add_tests_output
)
print(output)
