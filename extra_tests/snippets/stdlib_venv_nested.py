"""Regress RustPython venv aliases selecting the wrong base interpreter.

Standalone snippet: no third-party packages, network, or persistent files.
The runtime must provide venv and its usual subprocess/filesystem support.
"""

import os
import sys


def main():
    # This tests RustPython's native executable identity and its persisted base
    # metadata. Windows uses a separate launcher and Scripts/python.exe layout.
    if sys.implementation.name != "rustpython":
        print("SKIP: RustPython-specific venv base-executable regression")
        return
    if os.name != "posix" or sys.platform in {"emscripten", "wasi"}:
        print("SKIP: POSIX executable aliases and symlinks are required")
        return

    import json
    import pathlib
    import shutil
    import subprocess
    import tempfile
    import time

    deadline = time.monotonic() + 55.0
    child_env = dict(os.environ)
    for name in (
        "PYTHONHOME",
        "PYTHONPATH",
        "RUSTPYTHONPATH",
        "PYTHONEXECUTABLE",
        "__PYVENV_LAUNCHER__",
    ):
        child_env.pop(name, None)

    identity_code = (
        "import json, os, sys; "
        "print(json.dumps({"
        "'implementation': sys.implementation.name, "
        "'prefix': sys.prefix, "
        "'base': sys._base_executable, "
        "'base_exists': os.path.isfile(sys._base_executable)"
        "}))"
    )

    with tempfile.TemporaryDirectory(prefix="rustpython-venv-nested-") as tmp:
        work = pathlib.Path(tmp)
        base_home = work / "base-home"
        base_home.mkdir()
        unrelated_cwd = work / "unrelated-cwd"
        unrelated_cwd.mkdir()

        # A nonstandard primary name makes guessed home/rustpython or
        # home/python3 names insufficient; venv must preserve its actual base.
        base = base_home / "rustpython-base"
        source = os.path.realpath(sys.executable)
        try:
            os.link(source, base)
        except OSError:
            shutil.copy2(source, base)

        # This path exists and is executable, but is deliberately not our
        # interpreter. Missing-file-only fallback logic must not select it.
        decoy = base_home / "python"
        decoy.write_text(
            "#!/bin/sh\n"
            "printf '%s\\n' 'DECOY: selected unrelated home/python' >&2\n"
            "exit 86\n",
            encoding="utf-8",
        )
        os.chmod(decoy, 0o755)

        def run(command, *, expected_returncode=0):
            remaining = deadline - time.monotonic()
            assert remaining > 0, "venv nested regression exceeded its 55s budget"
            command = [str(arg) for arg in command]
            try:
                result = subprocess.run(
                    command,
                    cwd=unrelated_cwd,
                    env=child_env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    timeout=min(15.0, remaining),
                )
            except OSError as exc:
                raise AssertionError(f"Cannot launch {command!r}: {exc}") from exc
            assert result.returncode == expected_returncode, (
                f"Command {command!r} exited {result.returncode}; "
                f"expected {expected_returncode}\n"
                f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            )
            return result.stdout

        def check_identity(executable, prefix=None):
            result = json.loads(run([executable, "-I", "-c", identity_code]))
            assert result["implementation"] == "rustpython", result
            assert result["base_exists"], result
            assert os.path.realpath(result["base"]) == os.path.realpath(base), result
            if prefix is not None:
                assert os.path.realpath(result["prefix"]) == os.path.realpath(prefix), (
                    result
                )

        check_identity(base)
        run([decoy], expected_returncode=86)

        for outer_mode in ("copies", "symlinks"):
            outer = work / f"outer-{outer_mode}"
            run([base, "-I", "-m", "venv", "--without-pip", f"--{outer_mode}", outer])
            check_identity(outer / "bin" / base.name, outer)
            alias = outer / "bin" / "python"
            check_identity(alias, outer)

            nested_environments = []
            for nested_mode in ("copies", "symlinks"):
                nested = work / f"nested-{outer_mode}-{nested_mode}"
                # The common python alias is the trigger. Both modes must use
                # the original base even though unrelated home/python exists.
                run(
                    [
                        alias,
                        "-I",
                        "-m",
                        "venv",
                        "--without-pip",
                        f"--{nested_mode}",
                        nested,
                    ]
                )
                check_identity(nested / "bin" / base.name, nested)
                check_identity(nested / "bin" / "python", nested)
                nested_environments.append(nested)

            # A copied nested environment's creator metadata names its parent
            # venv. Stable base metadata must remain usable after that parent
            # and its copied interpreter are gone.
            shutil.rmtree(outer)
            for nested in nested_environments:
                check_identity(nested / "bin" / base.name, nested)
                check_identity(nested / "bin" / "python", nested)
                shutil.rmtree(nested)
            print(
                f"PASS: outer {outer_mode}; nested copies/symlinks survive parent deletion"
            )

    print("PASS: nested venv aliases preserve the RustPython base interpreter")


if __name__ == "__main__":
    main()
