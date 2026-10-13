"""Verify exact final Python metadata; refresh it only with scripts/update_lib."""

import os
from pathlib import Path

from target_source import verify_source

from utils import ROOT


def main():
    source = Path(os.environ["CPYTHON_ROOT"]).expanduser().resolve()
    verify_source(source)
    for name in ("opcode.py", "_opcode_metadata.py", "dis.py"):
        relative = Path("Lib") / name
        if (ROOT / relative).read_bytes() != (source / relative).read_bytes():
            raise ValueError(
                f"{relative} differs from pinned final source; use scripts/update_lib"
            )
        print(f"Verified {relative}")


if __name__ == "__main__":
    main()
