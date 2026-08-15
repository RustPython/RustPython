import argparse
import os
import shutil
import subprocess
import sys

from dataclasses import dataclass
from pathlib import Path
from typing import List

def subprocess_run(*args, **kwargs):
    # print('Process run:', *args, kwargs, file=sys.stderr)
    return subprocess.run(*args, **kwargs)

@dataclass
class LibEntry:
    name: str
    lib_exist: bool                 # Checks whether the library file existed in RustPython/Lib prior to testing
    test_exist: bool                # Checks whether the test file existed in RustPython/Lib/test prior to testing 
    test_do: bool                   # Checks if both the library file and the test file existed in cpython and therefore is possible to test
    test_ok: bool                   # Checks if the test returned OK or not
    path_cpython_lib: str
    path_cpython_test: str
    path_rpython_lib: str
    path_rpython_test: str
    path_tpython_lib: str
    path_tpython_test: str

    def __init__(self, name: str, CPYTHON_PATH: str, RPYTHON_PATH: str):
        self.name = name

        self.path_cpython_lib = os.path.join(CPYTHON_PATH, "Lib", f"{self.name}.py")
        self.path_rpython_lib = os.path.join(RPYTHON_PATH, "Lib", f"{self.name}.py")
        self.path_tpython_lib = os.path.join(RPYTHON_PATH, "LibTest", f"{self.name}.py")

        self.path_cpython_test = os.path.join(CPYTHON_PATH, "Lib", "test", f"test_{self.name}.py")
        self.path_rpython_test = os.path.join(RPYTHON_PATH, "Lib", "test", f"test_{self.name}.py")
        self.path_tpython_test = os.path.join(RPYTHON_PATH, "LibTest", "test", f"test_{self.name}.py")

        self.lib_exist = os.path.isfile(self.path_tpython_lib)
        self.test_exist = os.path.isfile(self.path_tpython_test)
        self.test_do = os.path.isfile(self.path_cpython_lib) and os.path.isfile(self.path_cpython_test)

    def run(self, RUSTEXECPATH: str):
        if self.test_do:
            shutil.copyfile(self.path_cpython_lib, self.path_tpython_lib)
            shutil.copyfile(self.path_cpython_test, self.path_tpython_test)

            result = subprocess_run(
                [RUSTEXECPATH, "-q", self.path_tpython_test],
                stdout=subprocess.PIPE, 
                stderr=subprocess.STDOUT,
                env={"RUSTPYTHONPATH": "LibTest"}
            )
            result = result.stdout.decode("utf-8")

            self.test_ok = "OK" in result

            if self.lib_exist:
                shutil.copyfile(self.path_rpython_lib, self.path_tpython_lib)
            else:
                os.remove(self.path_tpython_lib)
            
            if self.test_exist:
                shutil.copyfile(self.path_rpython_test, self.path_tpython_test)
            else:
                os.remove(self.path_tpython_test)

    def to_string(self):
        message = [f"{self.name}:"]

        if self.test_do:
            if self.test_ok:
                message.append("OK")
            else:
                message.append("Failed")
        else:
            message.append("No cpython lib or test file")

        return ' '.join(message)

def main():
    CURRENT_PATH = Path(__file__)

    parser = argparse.ArgumentParser(description="Test cpython library")
    parser.add_argument(
        "--cpython", 
        nargs=1, 
        default=os.path.join(
            CURRENT_PATH.parents[3],
            "cpython"
        ),
        required=False,
        help="Local cpython path."
    )
    parser.add_argument(
        "--rustpython", 
        nargs=1, 
        default=CURRENT_PATH.parents[2],
        required=False, 
        help="Local RustPython path."
    )
    args = vars(parser.parse_args())
    CPYTHON_PATH = args["cpython"]
    RPYTHON_PATH = args["rustpython"]

    if isinstance(CPYTHON_PATH, list):
        CPYTHON_PATH = CPYTHON_PATH[0]
    if isinstance(RPYTHON_PATH, list):
        RPYTHON_PATH = RPYTHON_PATH[0]

    shutil.copytree(
        os.path.join(RPYTHON_PATH, "Lib"),
        os.path.join(RPYTHON_PATH, "LibTest")
    )

    library_list: List[LibEntry] = []

    with open(os.path.join(CURRENT_PATH.parent, "targets.txt"), 'r') as f:
        for line in f:
            line = line.split('#')[0]
            line = line.strip()
            if line:
                library_list.append(LibEntry(line, CPYTHON_PATH, RPYTHON_PATH))

    subprocess_run(["cargo", "build", "--release", "-q"])
    REXECPATH = os.path.join(CURRENT_PATH.parents[2], "target", "release", "rustpython")

    try:
        for entry in library_list:
            entry.run(REXECPATH)
            print(entry.to_string())
    finally:
        shutil.rmtree(os.path.join(RPYTHON_PATH, "LibTest"))

if __name__ == "__main__":
    main()