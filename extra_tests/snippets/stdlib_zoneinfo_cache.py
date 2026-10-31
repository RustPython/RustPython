import tempfile
from pathlib import Path
from zoneinfo import ZoneInfo, reset_tzpath

import _interpreters

# Fixed-offset UTC TZif, so the cache check does not need system zone data.
UTC = b"TZif2\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00UTC\x00TZif2\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x00\x00\x04\x00\x00\x00\x00\x00\x00UTC\x00\nUTC0\n"

with tempfile.TemporaryDirectory() as directory:
    Path(directory, "UTC").write_bytes(UTC)
    reset_tzpath([directory])
    main = ZoneInfo("UTC")
    assert main is ZoneInfo("UTC")

    interp = _interpreters.create()
    try:
        result = _interpreters.run_string(
            interp,
            """
from zoneinfo import ZoneInfo, reset_tzpath
reset_tzpath([tzpath])
first = ZoneInfo("UTC")
second = ZoneInfo("UTC")
assert first is second
assert id(first) != main_id
ZoneInfo.clear_cache()
""",
            {"tzpath": directory, "main_id": id(main)},
        )
    finally:
        _interpreters.destroy(interp)

    if result is not None:
        detail = getattr(result, "formatted", None) or repr(result)
        raise AssertionError(detail)
    assert main is ZoneInfo("UTC")
