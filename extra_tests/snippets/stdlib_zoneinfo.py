import io
import struct
from zoneinfo import ZoneInfo
from zoneinfo._zoneinfo import ZoneInfo as PyZoneInfo

from testutils import assert_raises

# Minimal TZif v3 data with no transitions and a POSIX rule in the footer.
header = (b"TZif3" + b"\0" * 15 + struct.pack(">6l", 0, 0, 0, 0, 0, 0)) * 2


def zone_from_rule(zone_type, rule):
    return zone_type.from_file(io.BytesIO(header + b"\n" + rule + b"\n"))


for zone_type in (ZoneInfo, PyZoneInfo):
    for rule in (
        b"<>5",
        b"AAA4<>,M3.2.0/2,M11.1.0/3",
        b"AAA",
        b"A",
        b"AA",
        b"B",
        b"AB C3",
        b" A B 3",
        b"AAA4BB B,J60/2,J300/2",
        b"AAA4BBB,M3.2X0,M11.1.0",
        b"AAA4BBB,M3.2.0,M11.1X0",
        b"AAA4BBB,M3.2-0,M11.1.0/3",
        b"AAA4BBB,M3.2.0/2,M11.1:0",
        b"AAA4BBB,J1_0,J300/2",
        b"AAA4BBB,J60/2,J30_0/2",
        b"AAA4BBB,1_0,J300/2",
        b"AAA4BBB,J+1,J300/2",
        b"AAA4BBB,J 1,J300/2",
        b"AAA4BBB, 1,J300/2",
        b"AAA4BBB,J0001,J300/2",
        b"AAA4BBB,0001,J300/2",
        "ABÀC3".encode(),
        "AAA4BBB,J١,J300/2".encode(),
    ):
        assert_raises(ValueError, zone_from_rule, zone_type, rule)

    # Quoting still permits nonempty numeric abbreviations.
    assert zone_from_rule(zone_type, b"<+05>-5").tzname(None) == "+05"
    for rule in (b"AAA4BBB,J001/2,J065/2", b"AAA4BBB,001/2,065/2"):
        zone_from_rule(zone_type, rule)
