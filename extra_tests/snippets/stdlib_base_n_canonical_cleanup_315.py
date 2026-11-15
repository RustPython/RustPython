"""ASCII85 canonical conversion must preserve buffer cleanup and read order."""

import binascii


class Exporter:
    def __init__(self, payload, release):
        self.payload = payload
        self.release = release

    def __buffer__(self, flags):
        return memoryview(self.payload)

    def __release_buffer__(self, view):
        self.release()


def test_canonical_failure_releases_data_before_ignorechars():
    payload = bytearray(b"@/")
    events = []

    def release_auxiliary():
        events.append("ignorechars")
        payload.extend(b"!")

    class FailingCanonical:
        def __bool__(self):
            events.append("bool")
            raise RuntimeError("canonical conversion failed")

    primary = Exporter(payload, lambda: events.append("data"))
    auxiliary = Exporter(b"", release_auxiliary)
    try:
        binascii.a2b_ascii85(
            primary, ignorechars=auxiliary, canonical=FailingCanonical()
        )
    except RuntimeError as exc:
        assert str(exc) == "canonical conversion failed"
    else:
        raise AssertionError("expected canonical truth conversion failure")
    assert events == ["bool", "data", "ignorechars"]
    assert payload == b"@/!"


def test_canonical_conversion_precedes_ignorechars_snapshot():
    ignorechars = bytearray(b"?")

    class MutatingCanonical:
        def __bool__(self):
            ignorechars[:] = b"\n"
            return False

    assert (
        binascii.a2b_ascii85(
            b"@/\n", ignorechars=ignorechars, canonical=MutatingCanonical()
        )
        == b"a"
    )


if __name__ == "__main__":
    test_canonical_failure_releases_data_before_ignorechars()
    test_canonical_conversion_precedes_ignorechars_snapshot()
