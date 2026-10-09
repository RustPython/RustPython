import io
import struct
from zoneinfo import ZoneInfo

from testutils import assert_raises

# Minimal TZif v3 data with no transitions and a POSIX rule in the footer.
header = (b"TZif3" + b"\0" * 15 + struct.pack(">6l", 0, 0, 0, 0, 0, 0)) * 2


def zone_from_rule(rule):
    return ZoneInfo.from_file(io.BytesIO(header + b"\n" + rule + b"\n"))


for rule in (b"<>5", b"AAA4<>,M3.2.0/2,M11.1.0/3"):
    assert_raises(ValueError, zone_from_rule, rule)

# Quoting still permits nonempty numeric abbreviations.
assert zone_from_rule(b"<+05>-5").tzname(None) == "+05"
