"""Temporary observation of the existing canonical manager worker output."""

import locale
import os
import runpy
import sys
import tempfile
from pathlib import Path


def main():
    if sys.platform != "win32" or sys.implementation.name != "rustpython":
        raise SystemExit("This capture launcher is for Windows RustPython CI")

    if "--dont-add-python-opts" not in sys.argv[1:]:
        raise SystemExit("Capture requires the existing CI no-reexec option")
    # Script startup uses the script directory; original -m startup uses cwd.
    # With safe_path enabled neither startup mode prepends that entry.
    if not sys.flags.safe_path:
        sys.path[0] = os.getcwd()

    from test.libregrtest.run_workers import WorkerThread

    original_create_stdout = WorkerThread.create_stdout
    capture_dir = Path(__file__).resolve().parent / "capture"
    manager_names = {
        "test.test_multiprocessing_spawn.test_manager",
        "test_multiprocessing_spawn.test_manager",
    }

    def create_stdout(self, stack):
        if self.test_name not in manager_names:
            return original_create_stdout(self, stack)
        capture_dir.mkdir(parents=True, exist_ok=True)
        stdout_file = tempfile.NamedTemporaryFile(
            mode="w+",
            encoding=locale.getencoding(),
            errors="backslashreplace",
            prefix="manager-worker-",
            suffix=".log",
            dir=capture_dir,
            delete=False,
        )
        stack.enter_context(stdout_file)
        print(
            "Preserving manager worker capture: " + Path(stdout_file.name).name,
            flush=True,
        )
        return stdout_file

    WorkerThread.create_stdout = create_stdout
    # sys.orig_argv still identifies this launcher; the required existing
    # --dont-add-python-opts option disables regrtest re-exec through it.
    # Keep the ordinary test package entry point, test arguments, exit status and
    # worker spawning configuration. No test method or timeout is changed.
    runpy.run_module("test", run_name="__main__", alter_sys=True)


if __name__ == "__main__":
    main()
