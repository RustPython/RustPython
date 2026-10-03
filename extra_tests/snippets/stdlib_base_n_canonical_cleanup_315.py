"""PEP 688 cleanup/evaluation order when canonical truth conversion runs."""

import binascii


class Exporter:
    def __init__(self, payload, release):
        self.payload = payload
        self.release = release

    def __buffer__(self, flags):
        return memoryview(self.payload)

    def __release_buffer__(self, view):
        self.release()


for name, encoded in [
    ("a2b_base64", b"YQ=="),
    ("a2b_base32", b"ME======"),
    ("a2b_base85", b"VE"),
    ("a2b_ascii85", b"@/"),
]:
    fn = getattr(binascii, name)
    payload = bytearray(encoded)
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
        fn(primary, ignorechars=auxiliary, canonical=FailingCanonical())
    except RuntimeError as exc:
        assert str(exc) == "canonical conversion failed"
    else:
        raise AssertionError("expected canonical truth conversion failure")
    assert events == ["bool", "data", "ignorechars"], (name, events)
    assert payload == encoded + b"!", (name, payload)

    ignorechars = bytearray(b"?")

    class MutatingCanonical:
        def __bool__(self):
            ignorechars[:] = b"\n"
            return False

    assert (
        fn(encoded + b"\n", ignorechars=ignorechars, canonical=MutatingCanonical())
        == b"a"
    )

print("canonical buffer cleanup/evaluation: 8 cases passed")
