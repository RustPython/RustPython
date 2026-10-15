"""Malformed venv metadata must not select another installed interpreter."""

import os
import sys

if sys.implementation.name == "rustpython" and os.name == "posix":
    import json
    import shutil
    import subprocess
    import tempfile
    from pathlib import Path

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        home = root / "base"
        home.mkdir()
        decoy = home / "python"
        decoy.write_text("#!/bin/sh\nexit 86\n")
        decoy.chmod(0o755)
        env = root / "env"
        (env / "bin").mkdir(parents=True)
        executable = env / "bin" / "python"
        shutil.copy2(os.path.realpath(sys.executable), executable)
        child_env = dict(os.environ)
        for name in (
            "PYTHONHOME",
            "PYTHONPATH",
            "RUSTPYTHONPATH",
            "PYTHONEXECUTABLE",
            "__PYVENV_LAUNCHER__",
        ):
            child_env.pop(name, None)
        for key in ("base-executable", "executable"):
            (env / "pyvenv.cfg").write_text(f"home = {home}\n{key} = {executable}\n")
            result = subprocess.run(
                [
                    str(executable),
                    "-I",
                    "-c",
                    "import json,sys;print(json.dumps(sys._base_executable))",
                ],
                env=child_env,
                text=True,
                capture_output=True,
                timeout=10,
                check=True,
            )
            assert json.loads(result.stdout) == str(executable), result.stdout
    print("PASS: cyclic venv metadata preserves interpreter identity")
