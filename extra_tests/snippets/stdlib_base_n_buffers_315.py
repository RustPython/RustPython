"""Regressions for Python buffer callbacks during native codec cleanup."""

import binascii


class Exporter:
    def __init__(self, payload, on_release=None):
        self.payload = payload
        self.on_release = on_release
        self.events = []

    def __buffer__(self, flags):
        self.events.append(("acquire", flags))
        return memoryview(self.payload)

    def __release_buffer__(self, view):
        self.events.append(("release", bytes(view)))
        if self.on_release:
            self.on_release()


def test_auxiliary_export_outlives_conversion():
    payload = bytearray(b"a")
    exporter = Exporter(
        binascii.BASE32_ALPHABET, lambda: payload.__setitem__(0, ord("z"))
    )
    result = binascii.b2a_base32(payload, alphabet=exporter)
    assert result == binascii.b2a_base32(b"a")
    assert payload == b"z"
    assert [event[0] for event in exporter.events] == ["acquire", "release"]


def check_release_order(alphabet, *, fails):
    releases = []
    payload = bytearray(b"a")

    def release_alphabet():
        releases.append("alphabet")
        payload.extend(b"!")

    primary = Exporter(payload, lambda: releases.append("data"))
    auxiliary = Exporter(alphabet, release_alphabet)
    if fails:
        try:
            binascii.b2a_base32(primary, alphabet=auxiliary)
        except ValueError:
            pass
        else:
            raise AssertionError("codec unexpectedly succeeded")
    else:
        result = binascii.b2a_base32(primary, alphabet=auxiliary)
        assert result == binascii.b2a_base32(b"a")
    assert releases == ["data", "alphabet"]
    assert payload == b"a!"


def test_success_releases_data_before_auxiliary():
    check_release_order(binascii.BASE32_ALPHABET, fails=False)


def test_invalid_alphabet_releases_data_before_auxiliary():
    check_release_order(b"invalid", fails=True)


if __name__ == "__main__":
    test_auxiliary_export_outlives_conversion()
    test_success_releases_data_before_auxiliary()
    test_invalid_alphabet_releases_data_before_auxiliary()
