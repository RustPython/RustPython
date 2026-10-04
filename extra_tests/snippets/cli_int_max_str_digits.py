import subprocess
import sys

for option in (
    "int_max_str_digits",
    "int_max_str_digits=639",
    "int_max_str_digits=invalid",
):
    result = subprocess.run(
        [sys.executable, "-E", "-X", option, "-c", "pass"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert result.returncode == 1, (option, result.returncode, result.stderr)
    assert b"-X int_max_str_digits: invalid limit" in result.stderr, (
        option,
        result.stderr,
    )
    assert b"panicked" not in result.stderr, (option, result.stderr)

for digits in (640, 0):
    result = subprocess.run(
        [
            sys.executable,
            "-E",
            "-X",
            f"int_max_str_digits={digits}",
            "-c",
            "import sys; print(sys.get_int_max_str_digits())",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert result.returncode == 0, (digits, result.returncode, result.stderr)
    assert result.stdout.strip() == str(digits).encode(), (digits, result.stdout)
