import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path
import time

def check_cpython_path(pwd):
    pwd = Path(pwd)
    if not pwd.exists():
        raise FileNotFoundError(f"Path {pwd} does not exist")
    if not pwd.is_dir():
        raise FileNotFoundError(f"Path {pwd} is not a directory")
    if not (pwd / "Lib").exists():
        raise FileNotFoundError(f"Path {pwd} does not contain a 'Lib' directory")
    if not (pwd / "Lib").is_dir():
        raise FileNotFoundError(f"Path {pwd} contains a 'Lib' file, not a directory")
    # TODO: ensure dir is not rustpython

# Create the parser
parser = argparse.ArgumentParser()
# Add an argument
parser.add_argument('--cpy', type=str, required=True)
parser.add_argument('--verbose', action='store_true')
parser.add_argument('--dry-run', action='store_true')
parser.add_argument('--careful', action='store_true')
# Parse the argument
args = parser.parse_args()
# Print "Hello" + the user input argument
print("RUNNING UPGRADER")
implementation = platform.python_implementation()
if implementation != "CPython":
    sys.exit(f"update_stdlib.py must be run under CPython, got {implementation} instead")
print(f"Checking cpython location at {args.cpy}")
check_cpython_path(args.cpy)
cpy = Path(args.cpy)

print("Building rustpython")
features = ["encodings", "ssl"]
if not args.dry_run:
    subprocess.run(["cargo", "build", "--release", "--features=" + ",".join(features)], check=True)

# TODO: this is platform dependent
skips = ["test_bz2", "test_glob", "test_io", "test_os", "test_rlcompleter", "test_pathlib", "test_posixpath", "test_venv"]

# TODO: Uncomment this
# TODO: check to make sure nothing is staged or dirty in the git repo
# print("Running initial test")
# if not args.dry_run:
    # subprocess.run(["cargo", "run", "--release", "--features=" + ",".join(features), "--", "-m", "test", "-x"] + skips, check=True)


# get all the files in the cpython Lib directory
cpy_lib_files = list((cpy / "Lib").glob("*.py"))
cpy_test_files = list((cpy / "Lib/test").glob("*.py"))

cpy_lib_test_paris = []
non_pairs = cpy_test_files
for lib_file in cpy_lib_files:
    test_file = cpy / "Lib/test" / ("test_" + str(lib_file.relative_to(cpy / "Lib")))
    if test_file.exists():
        cpy_lib_test_paris.append((lib_file, test_file))
        non_pairs.remove(test_file)

print(f"Found {len(cpy_lib_test_paris)} test files")
if args.verbose:
    for lib_file, test_file in cpy_lib_test_paris:
        print(f"{lib_file} -> {test_file}")

print("Attempting upgrade of pairs")
run_base = ["cargo", "run", "--release", "--features=" + ",".join(features), "--", "-m", "test", ]
careful_run = run_base.copy() + ["-x"] + skips
for count, (lib_file, test_file) in enumerate(cpy_lib_test_paris):
    if test_file.name not in skips:
        print(f"[{count + 1}/{len(cpy_lib_test_paris)}] Upgrading {lib_file} and {test_file}")
        if not args.dry_run:
            # Copy current files to a backup location
            shutil.copy(lib_file, lib_file.with_suffix(".temp"))
            shutil.copy(test_file, test_file.with_suffix(".temp"))
            time.sleep(0.1)
            # Copy the files
            shutil.copy(lib_file, "Lib/")
            shutil.copy(test_file, "Lib/test/")
            time.sleep(0.1)
            run = run_base.copy() + [test_file.name.replace(".py", "")]
            if args.careful:
                run = careful_run
            # Run the tests, don't fail, but print the output if verbose and revert if failed
            try:
                subprocess.run(run, check=True)
            except subprocess.CalledProcessError as e:
                time.sleep(1)
                print(f"Test failed, reverting changes to {lib_file} and {test_file}")
                shutil.copy(lib_file.with_suffix(".temp"), lib_file)
                shutil.copy(test_file.with_suffix(".temp"), test_file)
                time.sleep(0.1)
                if args.verbose:
                    print(e)                
    else:
        print(f"Skipping {test_file}")
        continue

print("Attempting upgrade of non-pairs")
for count, test_file in enumerate(non_pairs):
    print(f"[{count + 1}/{len(non_pairs)}] Upgrading {test_file}")
    if not args.dry_run:
        # Copy current files to a backup location
        shutil.copy(test_file, test_file.with_suffix(".temp"))
        time.sleep(0.1)
        # Copy the files
        shutil.copy(test_file, "Lib/test/")
        time.sleep(0.1)
        run = run_base.copy() + [test_file.name.replace(".py", "")]
        if args.careful:
            run = careful_run
        # Run the tests, don't fail, but print the output if verbose and revert if failed
        try:
            subprocess.run(run, check=True)
        except subprocess.CalledProcessError as e:
            time.sleep(0.1)
            print(f"Test failed, reverting changes to {test_file}")
            shutil.copy(test_file.with_suffix(".temp"), test_file)
            time.sleep(0.1)
            if args.verbose:
                print(e) 
